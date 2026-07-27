import {
  ProductApiSchemaError,
  parseApiErrorResponse,
  parseProductPreferences,
  type ProductApprovalPreference,
  type ProductPreferences,
} from "../product/product-api-types";
import { ProductApiError } from "../product/product-client";
import {
  parseProductMemoryTopicContentResponse,
  parseProductMemoryTopicsResponse,
  parseProductRuntimeInfo,
  parseSettingsPreferencesUpdateRequest,
  validateProductMemorySlug,
  validateProductMemoryWorkspaceId,
  type ProductMemoryTopicContentResponse,
  type ProductMemoryTopicsResponse,
  type ProductRuntimeInfo,
  type SettingsPreferencesUpdateRequest,
} from "./settings-platform-api-types";

const DEFAULT_API_PREFIX = "/api";
const MAX_SETTINGS_RESPONSE_BYTES = 1_048_576;

export interface SettingsPlatformClientOptions {
  fetch?: typeof globalThis.fetch;
  /** Browser calls stay relative so the Next proxy owns upstream auth. */
  apiPrefix?: string;
}

export interface SettingsPlatformRequestOptions {
  signal?: AbortSignal;
}

export interface SettingsPlatformClient {
  listMemoryTopics(
    workspaceId: string,
    options?: SettingsPlatformRequestOptions,
  ): Promise<ProductMemoryTopicsResponse>;
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
  parse: (value: unknown) => T,
): Promise<T> {
  const response = await fetchImpl(url, init);
  if (!response.ok) {
    return throwPlatformApiError(response);
  }
  return parse(await readUnknownJson(response));
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

function getRequest(signal?: AbortSignal): RequestInit | undefined {
  return signal === undefined ? undefined : { signal };
}

export function createSettingsPlatformClient(
  options: SettingsPlatformClientOptions = {},
): SettingsPlatformClient {
  const fetchImpl = options.fetch ?? globalThis.fetch;
  if (!fetchImpl) {
    throw new Error("fetch is required to create a settings platform client");
  }
  const apiPrefix = normalizeApiPrefix(options.apiPrefix ?? DEFAULT_API_PREFIX);

  const updatePreferences = async (
    input: SettingsPreferencesUpdateRequest,
    requestOptions?: SettingsPlatformRequestOptions,
  ): Promise<ProductPreferences> => {
    const request = parseSettingsPreferencesUpdateRequest(input);
    const response = await requestJson(
      fetchImpl,
      productUrl(apiPrefix, "/product/preferences"),
      {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(request),
        signal: requestOptions?.signal,
      },
      parseProductPreferences,
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
      response.provider_selection !== undefined &&
      response.provider_selection.approval !== response.default_approval_policy
    ) {
      throw new ProductApiSchemaError(
        "settings preference response provider approval must match the default policy",
      );
    }
    return response;
  };

  return {
    listMemoryTopics(workspaceId, requestOptions) {
      return requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/memory/topics?${memoryWorkspaceQuery(workspaceId)}`,
        ),
        getRequest(requestOptions?.signal),
        parseProductMemoryTopicsResponse,
      );
    },

    async getMemoryTopic(workspaceId, slug, requestOptions) {
      const response = await requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/memory/topics/${encodeURIComponent(
            validateProductMemorySlug(slug),
          )}?${memoryWorkspaceQuery(workspaceId)}`,
        ),
        getRequest(requestOptions?.signal),
        parseProductMemoryTopicContentResponse,
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

    getRuntimeInfo(requestOptions) {
      return requestJson(
        fetchImpl,
        productUrl(apiPrefix, "/product/runtime"),
        getRequest(requestOptions?.signal),
        parseProductRuntimeInfo,
      );
    },

    getPreferences(requestOptions) {
      return requestJson(
        fetchImpl,
        productUrl(apiPrefix, "/product/preferences"),
        getRequest(requestOptions?.signal),
        parseProductPreferences,
      );
    },

    updatePreferences,

    updateDefaultApprovalPolicy(current, policy, requestOptions) {
      const preferences = parseProductPreferences(current);
      return updatePreferences(
        {
          schema_version: preferences.schema_version,
          expected_revision: preferences.revision,
          theme: preferences.theme,
          default_approval_policy: policy,
          active_workspace_id: preferences.active_workspace_id,
          active_session_id: preferences.active_session_id,
          provider_selection: preferences.provider_selection,
        },
        requestOptions,
      );
    },
  };
}
