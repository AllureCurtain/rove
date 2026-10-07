/**
 * Settings platform API contract types, generated-facing.
 *
 * Every wire record here is a compile-time alias of `apps/api/openapi.json`
 * through `generated/api-types.ts` (`pnpm gen:api-types`;
 * `pnpm check:api-types` fails on drift). What remains hand-written:
 * - `export const` enum arrays — runtime values the spec cannot express
 *   (asserted exact against the generated unions below);
 * - request validators and semantic guards (`validate*`, `parse*Request`,
 *   `parseSettingsPreferencesUpdateRequest`) — the client still refuses
 *   requests the server would reject, and the settings CAS transform is a
 *   client-side semantic, not a wire shape;
 * - `ProductMemoryListFilters` and `SettingsPreferencesUpdateRequest` —
 *   client-side shapes with no OpenAPI component.
 *
 * Response parsing for trusted same-origin JSON is retired: the generated
 * types are the contract.
 */
import type { components } from "../generated/api-types";
import {
  PRODUCT_APPROVAL_PREFERENCES,
  ProductApiSchemaError,
  type ProductApprovalPreference,
  type ProductPreferences,
  type ProductProviderSelection,
  type ProductSessionId,
  type ProductThemePreference,
  type ProductWorkspaceId,
  type UpdateProductPreferencesRequest,
} from "../product/product-api-types";

/** Schema lookup helper: `Schema<"X">` is the wire type of one OpenAPI component. */
type Schema<K extends keyof components["schemas"]> = components["schemas"][K];

/** Compile-time exact-match assertion between a const array and a union type. */
type AssertExact<T extends true> = T;
export const PRODUCT_MEMORY_TYPES = [
  "user",
  "feedback",
  "project",
  "reference",
] as const;
export type ProductMemoryType = Schema<"ProductMemoryType">;
export const PRODUCT_MEMORY_SCOPES = [
  "global",
  "project",
  "session",
] as const;
export type ProductMemoryScope = Schema<"ProductMemoryScope">;
export const PRODUCT_MEMORY_LAYERS = ["durable"] as const;
export type ProductMemoryLayer = Schema<"ProductMemoryLayer">;
export const PRODUCT_MEMORY_SOURCES = [
  "product_settings",
  "llm_tool",
  "other",
  "unknown",
] as const;
export type ProductMemorySource = Schema<"ProductMemorySource">;

export type ProductMemoryTopic = Schema<"ProductMemoryTopic">;

export type ProductMemoryTopicsResponse = Schema<"ProductMemoryTopicsResponse">;

export type ProductMemoryTopicContentResponse =
  Schema<"ProductMemoryTopicContentResponse">;

export interface ProductMemoryListFilters {
  q?: string;
  memory_type?: ProductMemoryType;
  scope?: ProductMemoryScope;
  source?: ProductMemorySource;
}

export type CreateProductMemoryTopicRequest =
  Schema<"CreateProductMemoryTopicRequest">;

export type UpdateProductMemoryTopicRequest =
  Schema<"UpdateProductMemoryTopicRequest">;

export const PRODUCT_MCP_TRANSPORTS = ["stdio", "sse", "streamable_http"] as const;
export type ProductMcpTransport = Schema<"ProductMcpTransport">;

/** The wire name is `ProductMcpServer`; the historical export name is kept. */
export type ProductMcpServerConfig = Schema<"ProductMcpServer">;

export type ProductMcpServersResponse = Schema<"ProductMcpServersResponse">;

export const PRODUCT_MCP_HEALTH_STATUSES = [
  "ready",
  "degraded",
  "disabled",
  "unknown",
] as const;
export type ProductMcpHealthStatus = Schema<"ProductMcpHealthStatus">;

export type ProductMcpHealthSnapshot = Schema<"ProductMcpHealthSnapshot">;

export type ProductMcpHealthResponse = Schema<"ProductMcpHealthResponse">;

export type CreateProductMcpServerRequest =
  Schema<"CreateProductMcpServerRequest">;

export type UpdateProductMcpServerRequest =
  Schema<"UpdateProductMcpServerRequest">;

export type ProductMcpToolDescriptor = Schema<"ProductMcpToolDescriptor">;

export type ProductMcpProbeResponse = Schema<"ProductMcpProbeResponse">;

export const PRODUCT_TRUST_STATES = [
  "unknown",
  "restricted",
  "trusted",
  "revoked",
] as const;
export type ProductTrustState = Schema<"ProductTrustState">;

export const PRODUCT_TRUST_CAPABILITIES = [
  "project_configuration",
  "workspace_instructions",
  "mcp_processes",
  "hooks_extensions",
  "provider_credentials",
  "external_paths",
] as const;
export type ProductTrustCapability = Schema<"ProductTrustCapability">;

export const PRODUCT_TRUST_DECISIONS = ["grant", "deny", "revoke"] as const;
export type ProductTrustDecision = Schema<"ProductTrustDecision">;

export type ProductTrustStatus = Schema<"ProductTrustStatus">;

export type ProductTrustDecisionRequest =
  Schema<"ProductTrustDecisionRequest">;

export type ProductConnectionStatus = Schema<"ProductConnectionStatus">;
export type ProductStoreStatus = Schema<"ProductStoreStatus">;
export type ProductResumeHealthStatus = Schema<"ProductResumeHealthStatus">;
export type ProductExecutionAdapter = Schema<"ProductExecutionAdapter">;
export type ProductExecutionWorkspaceKind =
  Schema<"ProductExecutionWorkspaceKind">;

export type ProductExecutionCapabilities =
  Schema<"ProductExecutionCapabilities">;

export type ProductExecutionEnvironmentInfo =
  Schema<"ProductExecutionEnvironmentInfo">;

export type ProductAgentRuntimeInfo = Schema<"ProductAgentRuntimeInfo">;

export type ProductResumeHealth = Schema<"ProductResumeHealth">;

export type ProductRuntimeInfo = Schema<"ProductRuntimeInfo">;

export interface SettingsPreferencesUpdateRequest {
  schema_version: number;
  expected_revision: number;
  theme: ProductThemePreference;
  default_approval_policy: ProductApprovalPreference;
  active_workspace_id?: ProductWorkspaceId;
  active_session_id?: ProductSessionId;
  provider_selection?: ProductProviderSelection;
}

type _AssertPRODUCT_MEMORY_TYPESExact = AssertExact<
  [(typeof PRODUCT_MEMORY_TYPES)[number]] extends [Schema<"ProductMemoryType">]
    ? [Schema<"ProductMemoryType">] extends [(typeof PRODUCT_MEMORY_TYPES)[number]]
      ? true
      : false
    : false
>;
type _AssertPRODUCT_MEMORY_SCOPESExact = AssertExact<
  [(typeof PRODUCT_MEMORY_SCOPES)[number]] extends [Schema<"ProductMemoryScope">]
    ? [Schema<"ProductMemoryScope">] extends [(typeof PRODUCT_MEMORY_SCOPES)[number]]
      ? true
      : false
    : false
>;
type _AssertPRODUCT_MEMORY_LAYERSExact = AssertExact<
  [(typeof PRODUCT_MEMORY_LAYERS)[number]] extends [Schema<"ProductMemoryLayer">]
    ? [Schema<"ProductMemoryLayer">] extends [(typeof PRODUCT_MEMORY_LAYERS)[number]]
      ? true
      : false
    : false
>;
type _AssertPRODUCT_MEMORY_SOURCESExact = AssertExact<
  [(typeof PRODUCT_MEMORY_SOURCES)[number]] extends [Schema<"ProductMemorySource">]
    ? [Schema<"ProductMemorySource">] extends [(typeof PRODUCT_MEMORY_SOURCES)[number]]
      ? true
      : false
    : false
>;
type _AssertPRODUCT_MCP_TRANSPORTSExact = AssertExact<
  [(typeof PRODUCT_MCP_TRANSPORTS)[number]] extends [Schema<"ProductMcpTransport">]
    ? [Schema<"ProductMcpTransport">] extends [(typeof PRODUCT_MCP_TRANSPORTS)[number]]
      ? true
      : false
    : false
>;
type _AssertPRODUCT_MCP_HEALTH_STATUSESExact = AssertExact<
  [(typeof PRODUCT_MCP_HEALTH_STATUSES)[number]] extends [Schema<"ProductMcpHealthStatus">]
    ? [Schema<"ProductMcpHealthStatus">] extends [(typeof PRODUCT_MCP_HEALTH_STATUSES)[number]]
      ? true
      : false
    : false
>;
type _AssertPRODUCT_TRUST_STATESExact = AssertExact<
  [(typeof PRODUCT_TRUST_STATES)[number]] extends [Schema<"ProductTrustState">]
    ? [Schema<"ProductTrustState">] extends [(typeof PRODUCT_TRUST_STATES)[number]]
      ? true
      : false
    : false
>;
type _AssertPRODUCT_TRUST_CAPABILITIESExact = AssertExact<
  [(typeof PRODUCT_TRUST_CAPABILITIES)[number]] extends [Schema<"ProductTrustCapability">]
    ? [Schema<"ProductTrustCapability">] extends [(typeof PRODUCT_TRUST_CAPABILITIES)[number]]
      ? true
      : false
    : false
>;
type _AssertPRODUCT_TRUST_DECISIONSExact = AssertExact<
  [(typeof PRODUCT_TRUST_DECISIONS)[number]] extends [Schema<"ProductTrustDecision">]
    ? [Schema<"ProductTrustDecision">] extends [(typeof PRODUCT_TRUST_DECISIONS)[number]]
      ? true
      : false
    : false
>;

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

/// Parses the fields a client may send. `transport_deprecated` is excluded on
/// purpose: it is server-owned and only present on responses.
function parseProductMcpServerFields(
  record: UnknownRecord,
  path: string,
): UpdateProductMcpServerRequest {
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
    required: expectBoolean(record.required, `${path}.required`),
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

export function parseCreateProductMcpServerRequest(
  value: unknown,
  path = "create product MCP server request",
): CreateProductMcpServerRequest {
  const record = expectRecord(value, path);
  // A create request is not a server config: it must not carry the
  // server-owned `transport_deprecated`, which the API rejects as unknown.
  expectOnlyKeys(
    record,
    [
      "name",
      "enabled",
      "required",
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

export function parseUpdateProductMcpServerRequest(
  value: unknown,
  path = "update product MCP server request",
): UpdateProductMcpServerRequest {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    [
      "enabled",
      "required",
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

export function parseSettingsPreferencesUpdateRequest(
  value: unknown,
  path = "settings preferences update request",
): SettingsPreferencesUpdateRequest {
  const request = value as UpdateProductPreferencesRequest;
  if (request.expected_revision == null) {
    return schemaError(`${path}.expected_revision`, "a required CAS revision");
  }
  if (request.expected_revision >= Number.MAX_SAFE_INTEGER) {
    return schemaError(
      `${path}.expected_revision`,
      `at most ${Number.MAX_SAFE_INTEGER - 1}`,
    );
  }
  if (request.default_approval_policy == null) {
    return schemaError(
      `${path}.default_approval_policy`,
      `one of ${PRODUCT_APPROVAL_PREFERENCES.join(", ")}`,
    );
  }
  if (request.theme == null) {
    return schemaError(
      `${path}.theme`,
      `one of ${["light", "dark", "system"].join(", ")}`,
    );
  }
  const settingsRequest: SettingsPreferencesUpdateRequest = {
    schema_version: request.schema_version ?? 0,
    expected_revision: request.expected_revision,
    theme: request.theme,
    default_approval_policy: request.default_approval_policy,
  };
  if (request.active_workspace_id != null) {
    settingsRequest.active_workspace_id = request.active_workspace_id;
  }
  if (request.active_session_id != null) {
    settingsRequest.active_session_id = request.active_session_id;
  }
  if (request.provider_selection != null) {
    settingsRequest.provider_selection = {
      ...request.provider_selection,
      approval: request.default_approval_policy,
    };
  }
  return settingsRequest;
}

export type { ProductApprovalPreference, ProductPreferences };
