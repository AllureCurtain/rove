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

export const PRODUCT_MEMORY_LAYERS = ["durable"] as const;
export type ProductMemoryLayer = (typeof PRODUCT_MEMORY_LAYERS)[number];

export const PRODUCT_MEMORY_SOURCES = [
  "product_settings",
  "llm_tool",
  "other",
  "unknown",
] as const;
export type ProductMemorySource = (typeof PRODUCT_MEMORY_SOURCES)[number];

export interface ProductMemoryTopic {
  slug: string;
  title: string;
  layer: ProductMemoryLayer;
  memory_type: ProductMemoryType;
  scope: ProductMemoryScope;
  source: ProductMemorySource;
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

export interface ProductMemoryListFilters {
  q?: string;
  memory_type?: ProductMemoryType;
  scope?: ProductMemoryScope;
  source?: ProductMemorySource;
}

export interface CreateProductMemoryTopicRequest {
  slug: string;
  title: string;
  memory_type: ProductMemoryType;
  scope: ProductMemoryScope;
  confidence: number;
  description: string;
  content: string;
}

export interface UpdateProductMemoryTopicRequest {
  title: string;
  memory_type: ProductMemoryType;
  scope: ProductMemoryScope;
  confidence: number;
  description: string;
  content: string;
  expected_updated_at?: string;
}

export const PRODUCT_MCP_TRANSPORTS = ["stdio", "sse"] as const;
export type ProductMcpTransport = (typeof PRODUCT_MCP_TRANSPORTS)[number];

export interface ProductMcpServerConfig {
  name: string;
  enabled: boolean;
  transport: ProductMcpTransport;
  command?: string;
  args: string[];
  env_names: string[];
  url?: string;
  request_timeout_ms: number;
}

export interface ProductMcpServersResponse {
  servers: ProductMcpServerConfig[];
  total: number;
}

export interface CreateProductMcpServerRequest {
  name: string;
  enabled: boolean;
  transport: ProductMcpTransport;
  command?: string;
  args: string[];
  env_names: string[];
  url?: string;
  request_timeout_ms: number;
}

export type UpdateProductMcpServerRequest = Omit<
  CreateProductMcpServerRequest,
  "name"
>;

export interface ProductMcpToolDescriptor {
  name: string;
  description: string;
  destructive: true;
  parallel_safe: false;
}

export interface ProductMcpProbeResponse {
  server_name: string;
  transport: ProductMcpTransport;
  tools: ProductMcpToolDescriptor[];
  tested_at: string;
}

export const PRODUCT_TRUST_STATES = [
  "unknown",
  "restricted",
  "trusted",
  "revoked",
] as const;
export type ProductTrustState = (typeof PRODUCT_TRUST_STATES)[number];

export const PRODUCT_TRUST_CAPABILITIES = [
  "project_configuration",
  "workspace_instructions",
  "mcp_processes",
  "hooks_extensions",
  "provider_credentials",
  "external_paths",
] as const;
export type ProductTrustCapability =
  (typeof PRODUCT_TRUST_CAPABILITIES)[number];

export const PRODUCT_TRUST_DECISIONS = ["grant", "deny", "revoke"] as const;
export type ProductTrustDecision = (typeof PRODUCT_TRUST_DECISIONS)[number];

export interface ProductTrustStatus {
  workspace_id: ProductWorkspaceId;
  state: ProductTrustState;
  identity_digest: string;
  invalidated_capabilities: ProductTrustCapability[];
  granted_capabilities: ProductTrustCapability[];
}

export interface ProductTrustDecisionRequest {
  decision: ProductTrustDecision;
  capabilities: ProductTrustCapability[];
}

export type ProductConnectionStatus = "connected";
export type ProductStoreStatus = "ready" | "unavailable";
export type ProductResumeHealthStatus = "healthy" | "needs_attention";
export type ProductExecutionAdapter = "local";
export type ProductExecutionWorkspaceKind = "folder" | "repo" | "task";

export interface ProductExecutionCapabilities {
  filesystem_read: boolean;
  filesystem_write: boolean;
  process_run: boolean;
  process_stdio: boolean;
  observations: boolean;
}

export interface ProductExecutionEnvironmentInfo {
  adapter: ProductExecutionAdapter;
  workspace_kind: ProductExecutionWorkspaceKind;
  workspace_digest: string;
  capabilities: ProductExecutionCapabilities;
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
const MAX_MEMORY_TOPIC_TITLE_BYTES = 256;
const MAX_MEMORY_TOPICS = 200;
const MAX_MEMORY_CONTENT_BYTES = 64 * 1_024;
const MAX_MCP_SERVERS = 32;
const MAX_MCP_SERVER_NAME_BYTES = 64;
const MAX_MCP_COMMAND_BYTES = 2_048;
const MAX_MCP_URL_BYTES = 2_048;
const MAX_MCP_ARGUMENTS = 64;
const MAX_MCP_ARGUMENT_BYTES = 2_048;
const MAX_MCP_ENV_NAMES = 32;
const MAX_MCP_TOOLS = 128;
const MAX_TRUST_DIGEST_BYTES = 256;
const SHA256_DIGEST_PATTERN = /^sha256:[0-9a-f]{64}$/u;
const MIN_MCP_TIMEOUT_MS = 100;
const MAX_MCP_TIMEOUT_MS = 120_000;
const MEMORY_TOPIC_SLUG_PATTERN =
  /^[\p{Alphabetic}\p{Number}]+(?:-[\p{Alphabetic}\p{Number}]+)*$/u;
const MCP_SERVER_NAME_PATTERN = /^[a-z0-9][a-z0-9_]*$/u;
const MCP_ENV_NAME_PATTERN = /^[A-Za-z_][A-Za-z0-9_]*$/u;
const MCP_SECRET_MARKERS = [
  "sk-",
  "bearer ",
  "api_key=",
  "api-key=",
  "token=",
  "password=",
  "secret=",
] as const;
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

function expectTrustCapabilities(
  value: unknown,
  path: string,
): ProductTrustCapability[] {
  if (!Array.isArray(value) || value.length > PRODUCT_TRUST_CAPABILITIES.length) {
    return schemaError(
      path,
      `an array with at most ${PRODUCT_TRUST_CAPABILITIES.length} items`,
    );
  }
  const capabilities = value.map((entry, index) =>
    expectEnum(
      entry,
      PRODUCT_TRUST_CAPABILITIES,
      `${path}[${index}]`,
    ),
  );
  if (new Set(capabilities).size !== capabilities.length) {
    return schemaError(path, "free of duplicate capabilities");
  }
  return capabilities;
}

export function parseProductTrustStatus(
  value: unknown,
  path = "product trust status",
): ProductTrustStatus {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    [
      "workspace_id",
      "state",
      "identity_digest",
      "invalidated_capabilities",
      "granted_capabilities",
    ],
    path,
  );
  const identityDigest = expectString(
    record.identity_digest,
    `${path}.identity_digest`,
    {
      nonEmpty: true,
      maxBytes: MAX_TRUST_DIGEST_BYTES,
      noControls: true,
    },
  );
  if (!SHA256_DIGEST_PATTERN.test(identityDigest)) {
    return schemaError(`${path}.identity_digest`, "a redacted sha256 digest");
  }
  return {
    workspace_id: expectString(record.workspace_id, `${path}.workspace_id`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControls: true,
    }),
    state: expectEnum(record.state, PRODUCT_TRUST_STATES, `${path}.state`),
    identity_digest: identityDigest,
    invalidated_capabilities: expectTrustCapabilities(
      record.invalidated_capabilities,
      `${path}.invalidated_capabilities`,
    ),
    granted_capabilities: expectTrustCapabilities(
      record.granted_capabilities,
      `${path}.granted_capabilities`,
    ),
  };
}

export function parseProductTrustDecisionRequest(
  value: ProductTrustDecisionRequest,
  path = "product trust decision request",
): ProductTrustDecisionRequest {
  const record = expectRecord(value, path);
  expectOnlyKeys(record, ["decision", "capabilities"], path);
  return {
    decision: expectEnum(
      record.decision,
      PRODUCT_TRUST_DECISIONS,
      `${path}.decision`,
    ),
    capabilities: expectTrustCapabilities(
      record.capabilities,
      `${path}.capabilities`,
    ),
  };
}

function expectMemoryTitle(value: unknown, path: string): string {
  const title = expectString(value, path, {
    nonEmpty: true,
    maxBytes: MAX_MEMORY_TOPIC_TITLE_BYTES,
    noControls: true,
  });
  if (title.includes("[") || title.includes("]")) {
    return schemaError(path, "free of memory-index delimiters");
  }
  return title;
}

function expectMemoryContent(value: unknown, path: string): string {
  const content = expectString(value, path, {
    maxBytes: MAX_MEMORY_CONTENT_BYTES,
  });
  if (content.includes("\0")) {
    return schemaError(path, "free of null bytes");
  }
  return content;
}

export function parseProductMemoryListFilters(
  value: ProductMemoryListFilters = {},
  path = "product memory filters",
): ProductMemoryListFilters {
  const record = expectRecord(value, path);
  expectOnlyKeys(record, ["q", "memory_type", "scope", "source"], path);
  const filters: ProductMemoryListFilters = {};
  if (record.q !== undefined) {
    const q = expectString(record.q, `${path}.q`, {
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControls: true,
    }).trim();
    if (q.length > 0) {
      filters.q = q;
    }
  }
  if (record.memory_type !== undefined) {
    filters.memory_type = expectEnum(
      record.memory_type,
      PRODUCT_MEMORY_TYPES,
      `${path}.memory_type`,
    );
  }
  if (record.scope !== undefined) {
    filters.scope = expectEnum(
      record.scope,
      PRODUCT_MEMORY_SCOPES,
      `${path}.scope`,
    );
  }
  if (record.source !== undefined) {
    filters.source = expectEnum(
      record.source,
      PRODUCT_MEMORY_SOURCES,
      `${path}.source`,
    );
  }
  return filters;
}

export function parseCreateProductMemoryTopicRequest(
  value: CreateProductMemoryTopicRequest,
  path = "create product memory topic request",
): CreateProductMemoryTopicRequest {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    [
      "slug",
      "title",
      "memory_type",
      "scope",
      "confidence",
      "description",
      "content",
    ],
    path,
  );
  return {
    slug: expectMemorySlug(record.slug, `${path}.slug`),
    title: expectMemoryTitle(record.title, `${path}.title`),
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
    content: expectMemoryContent(record.content, `${path}.content`),
  };
}

export function parseUpdateProductMemoryTopicRequest(
  value: UpdateProductMemoryTopicRequest,
  path = "update product memory topic request",
): UpdateProductMemoryTopicRequest {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    [
      "title",
      "memory_type",
      "scope",
      "confidence",
      "description",
      "content",
      "expected_updated_at",
    ],
    path,
  );
  const request: UpdateProductMemoryTopicRequest = {
    title: expectMemoryTitle(record.title, `${path}.title`),
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
    content: expectMemoryContent(record.content, `${path}.content`),
  };
  if (record.expected_updated_at !== undefined) {
    request.expected_updated_at = expectString(
      record.expected_updated_at,
      `${path}.expected_updated_at`,
      { maxBytes: MAX_PRODUCT_TEXT_BYTES, noControls: true },
    );
  }
  return request;
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
      "layer",
      "memory_type",
      "scope",
      "source",
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
    layer: expectEnum(
      record.layer,
      PRODUCT_MEMORY_LAYERS,
      `${path}.layer`,
    ),
    memory_type: expectEnum(
      record.memory_type,
      PRODUCT_MEMORY_TYPES,
      `${path}.memory_type`,
    ),
    scope: expectEnum(record.scope, PRODUCT_MEMORY_SCOPES, `${path}.scope`),
    source: expectEnum(
      record.source,
      PRODUCT_MEMORY_SOURCES,
      `${path}.source`,
    ),
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

function expectMcpServerName(value: unknown, path: string): string {
  const name = expectString(value, path, {
    nonEmpty: true,
    maxBytes: MAX_MCP_SERVER_NAME_BYTES,
    noControls: true,
  });
  if (!MCP_SERVER_NAME_PATTERN.test(name)) {
    return schemaError(
      path,
      "a lowercase identifier containing only letters, numbers, and underscores",
    );
  }
  return name;
}

export function validateProductMcpServerName(name: string): string {
  return expectMcpServerName(name, "product MCP server name");
}

function parseMcpStringArray(
  value: unknown,
  path: string,
  limit: number,
  itemLimit: number,
): string[] {
  if (!Array.isArray(value) || value.length > limit) {
    return schemaError(path, `an array with at most ${limit} items`);
  }
  return value.map((item, index) =>
    expectString(item, `${path}[${index}]`, {
      maxBytes: itemLimit,
      noControls: true,
    }),
  );
}

function parseMcpEnvironmentNames(value: unknown, path: string): string[] {
  const names = parseMcpStringArray(
    value,
    path,
    MAX_MCP_ENV_NAMES,
    MAX_PRODUCT_TEXT_BYTES,
  );
  const unique = new Set<string>();
  for (const name of names) {
    if (!MCP_ENV_NAME_PATTERN.test(name) || unique.has(name)) {
      return schemaError(path, "unique environment variable names");
    }
    unique.add(name);
  }
  return names;
}

function parseMcpArguments(value: unknown, path: string): string[] {
  const args = parseMcpStringArray(
    value,
    path,
    MAX_MCP_ARGUMENTS,
    MAX_MCP_ARGUMENT_BYTES,
  );
  if (
    args.some((argument) =>
      MCP_SECRET_MARKERS.some((marker) =>
        argument.toLocaleLowerCase("en-US").includes(marker),
      ),
    )
  ) {
    return schemaError(path, "free of raw secret-shaped values");
  }
  return args;
}

function expectMcpUrl(value: unknown, path: string): string {
  const url = expectString(value, path, {
    nonEmpty: true,
    maxBytes: MAX_MCP_URL_BYTES,
    noControls: true,
  });
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return schemaError(path, "an HTTP or HTTPS URL");
  }
  if (
    (parsed.protocol !== "http:" && parsed.protocol !== "https:") ||
    parsed.username !== "" ||
    parsed.password !== "" ||
    parsed.hash !== ""
  ) {
    return schemaError(path, "an HTTP or HTTPS URL without credentials or a fragment");
  }
  return url;
}

function parseProductMcpServerFields(
  record: UnknownRecord,
  path: string,
): Omit<ProductMcpServerConfig, "name"> {
  const transport = expectEnum(
    record.transport,
    PRODUCT_MCP_TRANSPORTS,
    `${path}.transport`,
  );
  const args = parseMcpArguments(record.args, `${path}.args`);
  const envNames = parseMcpEnvironmentNames(
    record.env_names,
    `${path}.env_names`,
  );
  const common = {
    enabled: expectBoolean(record.enabled, `${path}.enabled`),
    transport,
    args,
    env_names: envNames,
    request_timeout_ms: expectInteger(
      record.request_timeout_ms,
      `${path}.request_timeout_ms`,
      { min: MIN_MCP_TIMEOUT_MS, max: MAX_MCP_TIMEOUT_MS },
    ),
  };
  if (transport === "stdio") {
    if (record.url !== undefined) {
      return schemaError(`${path}.url`, "omitted for stdio transport");
    }
    return {
      ...common,
      command: expectString(record.command, `${path}.command`, {
        nonEmpty: true,
        maxBytes: MAX_MCP_COMMAND_BYTES,
        noControls: true,
      }),
    };
  }
  if (
    record.command !== undefined ||
    args.length !== 0 ||
    envNames.length !== 0
  ) {
    return schemaError(path, "an SSE config without command, args, or environment names");
  }
  return { ...common, url: expectMcpUrl(record.url, `${path}.url`) };
}

export function parseProductMcpServerConfig(
  value: unknown,
  path = "product MCP server",
): ProductMcpServerConfig {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    [
      "name",
      "enabled",
      "transport",
      "command",
      "args",
      "env_names",
      "url",
      "request_timeout_ms",
    ],
    path,
  );
  return {
    name: expectMcpServerName(record.name, `${path}.name`),
    ...parseProductMcpServerFields(record, path),
  };
}

export function parseCreateProductMcpServerRequest(
  value: unknown,
  path = "create product MCP server request",
): CreateProductMcpServerRequest {
  return parseProductMcpServerConfig(value, path);
}

export function parseUpdateProductMcpServerRequest(
  value: unknown,
  path = "update product MCP server request",
): UpdateProductMcpServerRequest {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    [
      "enabled",
      "transport",
      "command",
      "args",
      "env_names",
      "url",
      "request_timeout_ms",
    ],
    path,
  );
  return parseProductMcpServerFields(record, path);
}

export function parseProductMcpServersResponse(
  value: unknown,
  path = "product MCP servers response",
): ProductMcpServersResponse {
  const record = expectRecord(value, path);
  expectOnlyKeys(record, ["servers", "total"], path);
  if (!Array.isArray(record.servers) || record.servers.length > MAX_MCP_SERVERS) {
    return schemaError(
      `${path}.servers`,
      `an array with at most ${MAX_MCP_SERVERS} items`,
    );
  }
  const servers = record.servers.map((server, index) =>
    parseProductMcpServerConfig(server, `${path}.servers[${index}]`),
  );
  if (new Set(servers.map((server) => server.name)).size !== servers.length) {
    return schemaError(`${path}.servers`, "servers with unique names");
  }
  const total = expectInteger(record.total, `${path}.total`, {
    min: 0,
    max: MAX_MCP_SERVERS,
  });
  if (total !== servers.length) {
    return schemaError(`${path}.total`, "equal to the number of servers");
  }
  return { servers, total };
}

function parseProductMcpToolDescriptor(
  value: unknown,
  path: string,
): ProductMcpToolDescriptor {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    ["name", "description", "destructive", "parallel_safe"],
    path,
  );
  const destructive = expectBoolean(record.destructive, `${path}.destructive`);
  const parallelSafe = expectBoolean(
    record.parallel_safe,
    `${path}.parallel_safe`,
  );
  if (!destructive || parallelSafe) {
    return schemaError(path, "a locally restricted MCP tool descriptor");
  }
  return {
    name: expectString(record.name, `${path}.name`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControls: true,
    }),
    description: expectString(record.description, `${path}.description`, {
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControls: true,
    }),
    destructive: true,
    parallel_safe: false,
  };
}

export function parseProductMcpProbeResponse(
  value: unknown,
  path = "product MCP probe response",
): ProductMcpProbeResponse {
  const record = expectRecord(value, path);
  expectOnlyKeys(record, ["server_name", "transport", "tools", "tested_at"], path);
  if (!Array.isArray(record.tools) || record.tools.length > MAX_MCP_TOOLS) {
    return schemaError(
      `${path}.tools`,
      `an array with at most ${MAX_MCP_TOOLS} items`,
    );
  }
  const testedAt = expectString(record.tested_at, `${path}.tested_at`, {
    nonEmpty: true,
    maxBytes: MAX_PRODUCT_TEXT_BYTES,
    noControls: true,
  });
  if (!Number.isFinite(Date.parse(testedAt))) {
    return schemaError(`${path}.tested_at`, "an ISO timestamp");
  }
  return {
    server_name: expectMcpServerName(
      record.server_name,
      `${path}.server_name`,
    ),
    transport: expectEnum(
      record.transport,
      PRODUCT_MCP_TRANSPORTS,
      `${path}.transport`,
    ),
    tools: record.tools.map((tool, index) =>
      parseProductMcpToolDescriptor(tool, `${path}.tools[${index}]`),
    ),
    tested_at: testedAt,
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
    [
      "api_version",
      "connection",
      "product_store",
      "execution_environment",
      "resume_health",
    ],
    path,
  );
  const environmentPath = `${path}.execution_environment`;
  const environment = expectRecord(record.execution_environment, environmentPath);
  expectOnlyKeys(
    environment,
    ["adapter", "workspace_kind", "workspace_digest", "capabilities"],
    environmentPath,
  );
  const capabilitiesPath = `${environmentPath}.capabilities`;
  const capabilities = expectRecord(environment.capabilities, capabilitiesPath);
  expectOnlyKeys(
    capabilities,
    [
      "filesystem_read",
      "filesystem_write",
      "process_run",
      "process_stdio",
      "observations",
    ],
    capabilitiesPath,
  );
  const workspaceDigest = expectString(
    environment.workspace_digest,
    `${environmentPath}.workspace_digest`,
    {
      nonEmpty: true,
      maxBytes: MAX_TRUST_DIGEST_BYTES,
      noControls: true,
    },
  );
  if (!SHA256_DIGEST_PATTERN.test(workspaceDigest)) {
    return schemaError(
      `${environmentPath}.workspace_digest`,
      "a redacted sha256 digest",
    );
  }
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
    execution_environment: {
      adapter: expectEnum(
        environment.adapter,
        ["local"] as const,
        `${environmentPath}.adapter`,
      ),
      workspace_kind: expectEnum(
        environment.workspace_kind,
        ["folder", "repo", "task"] as const,
        `${environmentPath}.workspace_kind`,
      ),
      workspace_digest: workspaceDigest,
      capabilities: {
        filesystem_read: expectBoolean(
          capabilities.filesystem_read,
          `${capabilitiesPath}.filesystem_read`,
        ),
        filesystem_write: expectBoolean(
          capabilities.filesystem_write,
          `${capabilitiesPath}.filesystem_write`,
        ),
        process_run: expectBoolean(
          capabilities.process_run,
          `${capabilitiesPath}.process_run`,
        ),
        process_stdio: expectBoolean(
          capabilities.process_stdio,
          `${capabilitiesPath}.process_stdio`,
        ),
        observations: expectBoolean(
          capabilities.observations,
          `${capabilitiesPath}.observations`,
        ),
      },
    },
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
