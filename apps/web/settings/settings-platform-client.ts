import {
  ProductApiSchemaError,
  parseApiErrorResponse,
  type ProductApprovalPreference,
  type ProductPreferences,
} from "../product/product-api-types";
import { ProductApiError } from "../product/product-client";
import {
  desktopTransport,
  withDesktopAuthorization,
} from "../platform/desktop-transport";
import {
  parseCreateProductMcpServerRequest,
  parseCreateProductMemoryTopicRequest,
  parseProductMemoryListFilters,
  parseProductTrustDecisionRequest,
  parseSettingsPreferencesUpdateRequest,
  parseUpdateProductMcpServerRequest,
  parseUpdateProductMemoryTopicRequest,
  validateProductMcpServerName,
  validateProductMemorySlug,
  validateProductMemoryWorkspaceId,
  type CreateProductMcpServerRequest,
  type CreateProductMemoryTopicRequest,
  type ProductMcpProbeResponse,
  type ProductMcpHealthResponse,
  type ProductMcpServerConfig,
  type ProductMcpServersResponse,
  type ProductMemoryListFilters,
  type ProductMemoryTopicContentResponse,
  type ProductMemoryTopicsResponse,
  type ProductRuntimeInfo,
  type ProductTrustDecisionRequest,
  type ProductTrustStatus,
  type SettingsPreferencesUpdateRequest,
  type UpdateProductMcpServerRequest,
  type UpdateProductMemoryTopicRequest,
} from "./settings-platform-api-types";

const DEFAULT_API_PREFIX = "/api";
const MAX_SETTINGS_RESPONSE_BYTES = 1_048_576;

export interface SettingsPlatformClientOptions {
  fetch?: typeof globalThis.fetch;
  /** Browser calls stay relative so the Next proxy owns upstream auth. */
  apiPrefix?: string;
  apiToken?: string;
}

export interface SettingsPlatformRequestOptions {
  signal?: AbortSignal;
}

export interface SettingsPlatformClient {
  getProjectTrust(
    workspaceId: string,
    options?: SettingsPlatformRequestOptions,
  ): Promise<ProductTrustStatus>;
  decideProjectTrust(
    workspaceId: string,
    request: ProductTrustDecisionRequest,
    options?: SettingsPlatformRequestOptions,
  ): Promise<ProductTrustStatus>;
  listMemoryTopics(
    workspaceId: string,
    filters?: ProductMemoryListFilters,
    options?: SettingsPlatformRequestOptions,
  ): Promise<ProductMemoryTopicsResponse>;
  createMemoryTopic(
    workspaceId: string,
    request: CreateProductMemoryTopicRequest,
    options?: SettingsPlatformRequestOptions,
  ): Promise<ProductMemoryTopicContentResponse>;
  updateMemoryTopic(
    workspaceId: string,
    slug: string,
    request: UpdateProductMemoryTopicRequest,
    options?: SettingsPlatformRequestOptions,
  ): Promise<ProductMemoryTopicContentResponse>;
  getMemoryTopic(
    workspaceId: string,
    slug: string,
    options?: SettingsPlatformRequestOptions,
  ): Promise<ProductMemoryTopicContentResponse>;
  deleteMemoryTopic(
    workspaceId: string,
    slug: string,
    options?: SettingsPlatformRequestOptions,
  ): Promise<void>;
  listMcpServers(
    workspaceId: string,
    options?: SettingsPlatformRequestOptions,
  ): Promise<ProductMcpServersResponse>;
  getMcpHealth(
    workspaceId: string,
    options?: SettingsPlatformRequestOptions,
  ): Promise<ProductMcpHealthResponse>;
  createMcpServer(
    workspaceId: string,
    request: CreateProductMcpServerRequest,
    options?: SettingsPlatformRequestOptions,
  ): Promise<ProductMcpServerConfig>;
  updateMcpServer(
    workspaceId: string,
    name: string,
    request: UpdateProductMcpServerRequest,
    options?: SettingsPlatformRequestOptions,
  ): Promise<ProductMcpServerConfig>;
  deleteMcpServer(
    workspaceId: string,
    name: string,
    options?: SettingsPlatformRequestOptions,
  ): Promise<void>;
  probeMcpServer(
    workspaceId: string,
    name: string,
    options?: SettingsPlatformRequestOptions,
  ): Promise<ProductMcpProbeResponse>;
  getRuntimeInfo(
    options?: SettingsPlatformRequestOptions,
  ): Promise<ProductRuntimeInfo>;
  getPreferences(
    options?: SettingsPlatformRequestOptions,
  ): Promise<ProductPreferences>;
  updatePreferences(
    request: SettingsPreferencesUpdateRequest,
    options?: SettingsPlatformRequestOptions,
  ): Promise<ProductPreferences>;
  updateDefaultApprovalPolicy(
    current: ProductPreferences,
    policy: ProductApprovalPreference,
    options?: SettingsPlatformRequestOptions,
  ): Promise<ProductPreferences>;
}

function normalizeApiPrefix(prefix: string): string {
  const trimmed = prefix.trim();
  if (!trimmed) {
    throw new Error("settings platform API prefix must not be empty");
  }
  return trimmed.endsWith("/") ? trimmed.slice(0, -1) : trimmed;
}

function productUrl(prefix: string, path: string): string {
  return `${prefix}${path}`;
}

function memoryWorkspaceQuery(workspaceId: string): string {
  return `workspace_id=${encodeURIComponent(
    validateProductMemoryWorkspaceId(workspaceId),
  )}`;
}

function mcpWorkspaceQuery(workspaceId: string): string {
  return `workspace_id=${encodeURIComponent(
    validateProductMemoryWorkspaceId(workspaceId),
  )}`;
}

function memoryListQuery(
  workspaceId: string,
  input: ProductMemoryListFilters = {},
): string {
  const filters = parseProductMemoryListFilters(input);
  const query = new URLSearchParams();
  query.set(
    "workspace_id",
    validateProductMemoryWorkspaceId(workspaceId),
  );
  if (filters.q !== undefined) {
    query.set("q", filters.q);
  }
  if (filters.memory_type !== undefined) {
    query.set("memory_type", filters.memory_type);
  }
  if (filters.scope !== undefined) {
    query.set("scope", filters.scope);
  }
  if (filters.source !== undefined) {
    query.set("source", filters.source);
  }
  return query.toString();
}

async function readBoundedText(response: Response): Promise<string> {
  const text = await response.text();
  if (new TextEncoder().encode(text).length > MAX_SETTINGS_RESPONSE_BYTES) {
    throw new ProductApiSchemaError(
      `settings platform response must be at most ${MAX_SETTINGS_RESPONSE_BYTES} UTF-8 bytes`,
    );
  }
  return text;
}

async function readUnknownJson(response: Response): Promise<unknown> {
  const text = await readBoundedText(response);
  if (!text.trim()) {
    throw new ProductApiSchemaError(
      "settings platform response must contain JSON",
    );
  }
  try {
    return JSON.parse(text) as unknown;
  } catch {
    throw new ProductApiSchemaError(
      "settings platform response must be valid JSON",
    );
  }
}

async function throwPlatformApiError(response: Response): Promise<never> {
  const text = await readBoundedText(response);
  let payload: unknown = null;
  if (text.trim()) {
    try {
      payload = JSON.parse(text) as unknown;
    } catch {
      payload = null;
    }
  }
  const error = parseApiErrorResponse(payload);
  throw new ProductApiError(
    response.status,
    error?.code ?? "http_error",
    error?.error ??
      `settings platform request failed with status ${response.status}`,
  );
}

async function requestJson<T>(
  fetchImpl: typeof globalThis.fetch,
  url: string,
  init: RequestInit | undefined,
): Promise<T> {
  const response = await fetchImpl(url, init);
  if (!response.ok) {
    return throwPlatformApiError(response);
  }
  return (await readUnknownJson(response)) as T;
}

async function requestNoContent(
  fetchImpl: typeof globalThis.fetch,
  url: string,
  signal?: AbortSignal,
): Promise<void> {
  const response = await fetchImpl(url, { method: "DELETE", signal });
  if (!response.ok) {
    return throwPlatformApiError(response);
  }
  const body = await readBoundedText(response);
  if (response.status !== 204 || body.length !== 0) {
    throw new ProductApiSchemaError(
      `settings platform delete response must be an empty 204, received ${response.status}`,
    );
  }
}

async function requestMemoryMutation(
  fetchImpl: typeof globalThis.fetch,
  url: string,
  method: "POST" | "PUT",
  body: CreateProductMemoryTopicRequest | UpdateProductMemoryTopicRequest,
  expectedStatus: 200 | 201,
  signal?: AbortSignal,
): Promise<ProductMemoryTopicContentResponse> {
  const response = await fetchImpl(url, {
    method,
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
    signal,
  });
  if (!response.ok) {
    return throwPlatformApiError(response);
  }
  if (response.status !== expectedStatus) {
    throw new ProductApiSchemaError(
      `settings memory mutation expected status ${expectedStatus}, received ${response.status}`,
    );
  }
  return (await readUnknownJson(response)) as ProductMemoryTopicContentResponse;
}

function validateMemoryMutationResponse(
  response: ProductMemoryTopicContentResponse,
  slug: string,
  content: string,
): ProductMemoryTopicContentResponse {
  if (response.topic.slug !== slug || response.content !== content) {
    throw new ProductApiSchemaError(
      "product memory mutation response must match the requested slug and content",
    );
  }
  if (
    response.topic.layer !== "durable" ||
    response.topic.source !== "product_settings" ||
    response.truncated
  ) {
    throw new ProductApiSchemaError(
      "product memory mutation response must describe a complete durable settings write",
    );
  }
  return response;
}

function getRequest(signal?: AbortSignal): RequestInit | undefined {
  return signal === undefined ? undefined : { signal };
}

async function requestMcpMutation(
  fetchImpl: typeof globalThis.fetch,
  url: string,
  method: "POST" | "PUT",
  body: CreateProductMcpServerRequest | UpdateProductMcpServerRequest,
  expectedStatus: 200 | 201,
  signal?: AbortSignal,
): Promise<ProductMcpServerConfig> {
  const response = await fetchImpl(url, {
    method,
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
    signal,
  });
  if (!response.ok) {
    return throwPlatformApiError(response);
  }
  if (response.status !== expectedStatus) {
    throw new ProductApiSchemaError(
      `settings MCP mutation expected status ${expectedStatus}, received ${response.status}`,
    );
  }
  return (await readUnknownJson(response)) as ProductMcpServerConfig;
}

export function createSettingsPlatformClient(
  options: SettingsPlatformClientOptions = {},
): SettingsPlatformClient {
  const baseFetch = options.fetch ?? globalThis.fetch;
  if (!baseFetch) {
    throw new Error("fetch is required to create a settings platform client");
  }
  const desktop = desktopTransport();
  const fetchImpl = withDesktopAuthorization(
    baseFetch,
    options.apiToken ?? desktop?.token,
  );
  const apiPrefix = normalizeApiPrefix(
    options.apiPrefix ?? desktop?.apiPrefix ?? DEFAULT_API_PREFIX,
  );

  const updatePreferences = async (
    input: SettingsPreferencesUpdateRequest,
    requestOptions?: SettingsPlatformRequestOptions,
  ): Promise<ProductPreferences> => {
    const request = parseSettingsPreferencesUpdateRequest(input);
    const response = await requestJson<ProductPreferences>(
      fetchImpl,
      productUrl(apiPrefix, "/product/preferences"),
      {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(request),
        signal: requestOptions?.signal,
      },
    );
    if (response.revision !== request.expected_revision + 1) {
      throw new ProductApiSchemaError(
        "settings preference response revision must advance the expected revision by one",
      );
    }
    if (response.default_approval_policy !== request.default_approval_policy) {
      throw new ProductApiSchemaError(
        "settings preference response must contain the requested approval policy",
      );
    }
    if (
      (response.provider_selection !== undefined) !==
      (request.provider_selection !== undefined)
    ) {
      throw new ProductApiSchemaError(
        "settings preference response must preserve the requested provider selection",
      );
    }
    if (
      response.provider_selection != null &&
      response.provider_selection.approval !== response.default_approval_policy
    ) {
      throw new ProductApiSchemaError(
        "settings preference response provider approval must match the default policy",
      );
    }
    return response;
  };

  return {
    async getProjectTrust(workspaceId, requestOptions) {
      const validWorkspaceId = validateProductMemoryWorkspaceId(workspaceId);
      const response = await requestJson<ProductTrustStatus>(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/workspaces/${encodeURIComponent(validWorkspaceId)}/trust`,
        ),
        getRequest(requestOptions?.signal)
      );
      if (response.workspace_id !== validWorkspaceId) {
        throw new ProductApiSchemaError(
          "product trust response must match the requested workspace id",
        );
      }
      return response;
    },

    async decideProjectTrust(workspaceId, input, requestOptions) {
      const validWorkspaceId = validateProductMemoryWorkspaceId(workspaceId);
      const request = parseProductTrustDecisionRequest(input);
      const response = await requestJson<ProductTrustStatus>(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/workspaces/${encodeURIComponent(validWorkspaceId)}/trust`,
        ),
        {
          method: "PUT",
          headers: { "content-type": "application/json" },
          body: JSON.stringify(request),
          signal: requestOptions?.signal,
        }
      );
      if (response.workspace_id !== validWorkspaceId) {
        throw new ProductApiSchemaError(
          "product trust response must match the requested workspace id",
        );
      }
      return response;
    },

    listMemoryTopics(workspaceId, filters, requestOptions) {
      return requestJson<ProductMemoryTopicsResponse>(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/memory/topics?${memoryListQuery(workspaceId, filters)}`,
        ),
        getRequest(requestOptions?.signal)
      );
    },

    async createMemoryTopic(workspaceId, input, requestOptions) {
      const request = parseCreateProductMemoryTopicRequest(input);
      const response = await requestMemoryMutation(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/memory/topics?${memoryWorkspaceQuery(workspaceId)}`,
        ),
        "POST",
        request,
        201,
        requestOptions?.signal,
      );
      return validateMemoryMutationResponse(
        response,
        request.slug,
        request.content,
      );
    },

    async updateMemoryTopic(workspaceId, slug, input, requestOptions) {
      const validSlug = validateProductMemorySlug(slug);
      const request = parseUpdateProductMemoryTopicRequest(input);
      const response = await requestMemoryMutation(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/memory/topics/${encodeURIComponent(
            validSlug,
          )}?${memoryWorkspaceQuery(workspaceId)}`,
        ),
        "PUT",
        request,
        200,
        requestOptions?.signal,
      );
      return validateMemoryMutationResponse(response, validSlug, request.content);
    },

    async getMemoryTopic(workspaceId, slug, requestOptions) {
      const response = await requestJson<ProductMemoryTopicContentResponse>(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/memory/topics/${encodeURIComponent(
            validateProductMemorySlug(slug),
          )}?${memoryWorkspaceQuery(workspaceId)}`,
        ),
        getRequest(requestOptions?.signal)
      );
      if (response.topic.slug !== slug) {
        throw new ProductApiSchemaError(
          "product memory response must match the requested topic slug",
        );
      }
      return response;
    },

    deleteMemoryTopic(workspaceId, slug, requestOptions) {
      return requestNoContent(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/memory/topics/${encodeURIComponent(
            validateProductMemorySlug(slug),
          )}?${memoryWorkspaceQuery(workspaceId)}`,
        ),
        requestOptions?.signal,
      );
    },

    listMcpServers(workspaceId, requestOptions) {
      return requestJson<ProductMcpServersResponse>(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/mcp/servers?${mcpWorkspaceQuery(workspaceId)}`,
        ),
        getRequest(requestOptions?.signal)
      );
    },

    getMcpHealth(workspaceId, requestOptions) {
      return requestJson<ProductMcpHealthResponse>(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/mcp/health?${mcpWorkspaceQuery(workspaceId)}`,
        ),
        getRequest(requestOptions?.signal)
      );
    },

    async createMcpServer(workspaceId, input, requestOptions) {
      const request = parseCreateProductMcpServerRequest(input);
      const response = await requestMcpMutation(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/mcp/servers?${mcpWorkspaceQuery(workspaceId)}`,
        ),
        "POST",
        request,
        201,
        requestOptions?.signal,
      );
      if (response.name !== request.name) {
        throw new ProductApiSchemaError(
          "product MCP create response must match the requested server name",
        );
      }
      return response;
    },

    async updateMcpServer(workspaceId, name, input, requestOptions) {
      const validName = validateProductMcpServerName(name);
      const request = parseUpdateProductMcpServerRequest(input);
      const response = await requestMcpMutation(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/mcp/servers/${encodeURIComponent(
            validName,
          )}?${mcpWorkspaceQuery(workspaceId)}`,
        ),
        "PUT",
        request,
        200,
        requestOptions?.signal,
      );
      if (response.name !== validName) {
        throw new ProductApiSchemaError(
          "product MCP update response must match the requested server name",
        );
      }
      return response;
    },

    deleteMcpServer(workspaceId, name, requestOptions) {
      return requestNoContent(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/mcp/servers/${encodeURIComponent(
            validateProductMcpServerName(name),
          )}?${mcpWorkspaceQuery(workspaceId)}`,
        ),
        requestOptions?.signal,
      );
    },

    async probeMcpServer(workspaceId, name, requestOptions) {
      const validName = validateProductMcpServerName(name);
      const response = await requestJson<ProductMcpProbeResponse>(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/mcp/servers/${encodeURIComponent(
            validName,
          )}/probe?${mcpWorkspaceQuery(workspaceId)}`,
        ),
        { method: "POST", signal: requestOptions?.signal }
      );
      if (response.server_name !== validName) {
        throw new ProductApiSchemaError(
          "product MCP probe response must match the requested server name",
        );
      }
      return response;
    },

    getRuntimeInfo(requestOptions) {
      return requestJson<ProductRuntimeInfo>(
        fetchImpl,
        productUrl(apiPrefix, "/product/runtime"),
        getRequest(requestOptions?.signal)
      );
    },

    getPreferences(requestOptions) {
      return requestJson<ProductPreferences>(
        fetchImpl,
        productUrl(apiPrefix, "/product/preferences"),
        getRequest(requestOptions?.signal),
      );
    },

    updatePreferences,

    updateDefaultApprovalPolicy(current, policy, requestOptions) {
      const preferences = current;
      return updatePreferences(
        {
          schema_version: preferences.schema_version,
          expected_revision: preferences.revision ?? 0,
          theme: preferences.theme,
          default_approval_policy: policy,
          active_workspace_id:
            preferences.active_workspace_id ?? undefined,
          active_session_id: preferences.active_session_id ?? undefined,
          provider_selection: preferences.provider_selection ?? undefined,
        },
        requestOptions,
      );
    },
  };
}
