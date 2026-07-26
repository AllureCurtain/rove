import {
  ProductApiSchemaError,
  parseApiErrorResponse,
  parseCreateProductSessionRequest,
  parseCreateProductWorkspaceRequest,
  parseM1BrowserMigrationRequest,
  parseM1BrowserMigrationResponse,
  parseProductPreferences,
  parseProductProviderProfile,
  parseProductProviderProfileRequest,
  parseProductProviderProfilesResponse,
  parseProductSession,
  parseProductSessionsResponse,
  parseProductTranscriptResponse,
  parseProductWorkspace,
  parseProductWorkspacesResponse,
  parseUpdateProductPreferencesRequest,
  parseUpdateProductSessionRequest,
  type CreateProductProviderProfileRequest,
  type CreateProductSessionRequest,
  type CreateProductWorkspaceRequest,
  type M1BrowserMigrationRequest,
  type M1BrowserMigrationResponse,
  type ProductPreferences,
  type ProductProviderProfile,
  type ProductProviderProfilesResponse,
  type ProductSession,
  type ProductSessionsResponse,
  type ProductTranscriptResponse,
  type ProductWorkspace,
  type ProductWorkspacesResponse,
  type UpdateProductPreferencesRequest,
  type UpdateProductProviderProfileRequest,
  type UpdateProductSessionRequest,
} from "./product-api-types";

const DEFAULT_API_PREFIX = "/api";

export class ProductApiError extends Error {
  readonly status: number;
  readonly code: string;

  constructor(status: number, code: string, message: string) {
    super(message);
    this.name = "ProductApiError";
    this.status = status;
    this.code = code;
  }
}

export interface ProductApiClientOptions {
  fetch?: typeof globalThis.fetch;
  /** Browser calls stay relative so the Next proxy owns upstream auth. */
  apiPrefix?: string;
}

export interface ExactM1BrowserMigrationBody {
  request: M1BrowserMigrationRequest;
  body: string;
}

export interface ProductApiRequestOptions {
  signal?: AbortSignal;
}

export interface ProductApiClient {
  listWorkspaces(): Promise<ProductWorkspacesResponse>;
  createWorkspace(
    request: CreateProductWorkspaceRequest,
  ): Promise<ProductWorkspace>;
  deleteWorkspace(workspaceId: string): Promise<void>;
  listSessions(workspaceId: string): Promise<ProductSessionsResponse>;
  createSession(request: CreateProductSessionRequest): Promise<ProductSession>;
  updateSession(
    sessionId: string,
    request: UpdateProductSessionRequest,
  ): Promise<ProductSession>;
  deleteSession(sessionId: string): Promise<void>;
  getTranscript(sessionId: string): Promise<ProductTranscriptResponse>;
  listProviderProfiles(): Promise<ProductProviderProfilesResponse>;
  createProviderProfile(
    request: CreateProductProviderProfileRequest,
  ): Promise<ProductProviderProfile>;
  updateProviderProfile(
    profileId: string,
    request: UpdateProductProviderProfileRequest,
  ): Promise<ProductProviderProfile>;
  deleteProviderProfile(profileId: string): Promise<void>;
  getPreferences(): Promise<ProductPreferences>;
  updatePreferences(
    request: UpdateProductPreferencesRequest,
  ): Promise<ProductPreferences>;
  migrateM1BrowserState(
    exact: ExactM1BrowserMigrationBody,
    options?: ProductApiRequestOptions,
  ): Promise<M1BrowserMigrationResponse>;
}

function normalizeApiPrefix(prefix: string): string {
  const trimmed = prefix.trim();
  if (!trimmed) {
    throw new Error("product API prefix must not be empty");
  }
  return trimmed.endsWith("/") ? trimmed.slice(0, -1) : trimmed;
}

function productUrl(prefix: string, path: string): string {
  return `${prefix}${path}`;
}

async function readUnknownJson(response: Response): Promise<unknown> {
  const text = await response.text();
  if (!text.trim()) {
    throw new ProductApiSchemaError("product API response must contain JSON");
  }
  try {
    const parsed: unknown = JSON.parse(text);
    return parsed;
  } catch {
    throw new ProductApiSchemaError("product API response must be valid JSON");
  }
}

async function throwProductApiError(response: Response): Promise<never> {
  const text = await response.text();
  let payload: unknown = null;
  if (text.trim()) {
    try {
      payload = JSON.parse(text);
    } catch {
      payload = null;
    }
  }
  const error = parseApiErrorResponse(payload);
  throw new ProductApiError(
    response.status,
    error?.code ?? "http_error",
    error?.error ?? `product API request failed with status ${response.status}`,
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
    return throwProductApiError(response);
  }
  return parse(await readUnknownJson(response));
}

async function requestNoContent(
  fetchImpl: typeof globalThis.fetch,
  url: string,
): Promise<void> {
  const response = await fetchImpl(url, { method: "DELETE" });
  if (!response.ok) {
    return throwProductApiError(response);
  }
  if (response.status !== 204) {
    throw new ProductApiSchemaError(
      `product delete response must have status 204, received ${response.status}`,
    );
  }
}

function jsonRequest(
  method: "POST" | "PUT" | "PATCH",
  body: string,
  signal?: AbortSignal,
): RequestInit {
  return {
    method,
    headers: { "content-type": "application/json" },
    body,
    signal,
  };
}

function canonicalM1MigrationBody(exact: ExactM1BrowserMigrationBody): string {
  const request = parseM1BrowserMigrationRequest(exact.request);
  let parsedBody: unknown;
  try {
    parsedBody = JSON.parse(exact.body);
  } catch {
    throw new ProductApiSchemaError(
      "persisted M1 migration request body must be valid JSON",
    );
  }
  const bodyRequest = parseM1BrowserMigrationRequest(parsedBody);
  const canonicalRequest = JSON.stringify(request);
  if (
    JSON.stringify(bodyRequest) !== canonicalRequest ||
    exact.body !== canonicalRequest
  ) {
    throw new ProductApiSchemaError(
      "persisted M1 migration request body does not exactly match its request",
    );
  }
  return exact.body;
}

export function createProductApiClient(
  options: ProductApiClientOptions = {},
): ProductApiClient {
  const fetchImpl = options.fetch ?? globalThis.fetch;
  if (!fetchImpl) {
    throw new Error("fetch is required to create a product API client");
  }
  const apiPrefix = normalizeApiPrefix(options.apiPrefix ?? DEFAULT_API_PREFIX);

  return {
    listWorkspaces() {
      return requestJson(
        fetchImpl,
        productUrl(apiPrefix, "/product/workspaces"),
        undefined,
        parseProductWorkspacesResponse,
      );
    },

    async createWorkspace(input) {
      const request = parseCreateProductWorkspaceRequest(input);
      return requestJson(
        fetchImpl,
        productUrl(apiPrefix, "/product/workspaces"),
        jsonRequest("POST", JSON.stringify(request)),
        parseProductWorkspace,
      );
    },

    deleteWorkspace(workspaceId) {
      return requestNoContent(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/workspaces/${encodeURIComponent(workspaceId)}`,
        ),
      );
    },

    listSessions(workspaceId) {
      return requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/sessions?workspace_id=${encodeURIComponent(workspaceId)}`,
        ),
        undefined,
        parseProductSessionsResponse,
      );
    },

    async createSession(input) {
      const request = parseCreateProductSessionRequest(input);
      return requestJson(
        fetchImpl,
        productUrl(apiPrefix, "/product/sessions"),
        jsonRequest("POST", JSON.stringify(request)),
        parseProductSession,
      );
    },

    async updateSession(sessionId, input) {
      const request = parseUpdateProductSessionRequest(input);
      return requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/sessions/${encodeURIComponent(sessionId)}`,
        ),
        jsonRequest("PATCH", JSON.stringify(request)),
        parseProductSession,
      );
    },

    deleteSession(sessionId) {
      return requestNoContent(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/sessions/${encodeURIComponent(sessionId)}`,
        ),
      );
    },

    async getTranscript(sessionId) {
      const transcript = await requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/sessions/${encodeURIComponent(sessionId)}/transcript`,
        ),
        undefined,
        parseProductTranscriptResponse,
      );
      if (transcript.product_session_id !== sessionId) {
        throw new ProductApiSchemaError(
          "product transcript response must match the requested product session",
        );
      }
      return transcript;
    },

    listProviderProfiles() {
      return requestJson(
        fetchImpl,
        productUrl(apiPrefix, "/product/provider-profiles"),
        undefined,
        parseProductProviderProfilesResponse,
      );
    },

    async createProviderProfile(input) {
      const request = parseProductProviderProfileRequest(input);
      return requestJson(
        fetchImpl,
        productUrl(apiPrefix, "/product/provider-profiles"),
        jsonRequest("POST", JSON.stringify(request)),
        parseProductProviderProfile,
      );
    },

    async updateProviderProfile(profileId, input) {
      const request = parseProductProviderProfileRequest(input);
      return requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/provider-profiles/${encodeURIComponent(profileId)}`,
        ),
        jsonRequest("PUT", JSON.stringify(request)),
        parseProductProviderProfile,
      );
    },

    deleteProviderProfile(profileId) {
      return requestNoContent(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/provider-profiles/${encodeURIComponent(profileId)}`,
        ),
      );
    },

    getPreferences() {
      return requestJson(
        fetchImpl,
        productUrl(apiPrefix, "/product/preferences"),
        undefined,
        parseProductPreferences,
      );
    },

    async updatePreferences(input) {
      const request = parseUpdateProductPreferencesRequest(input);
      return requestJson(
        fetchImpl,
        productUrl(apiPrefix, "/product/preferences"),
        jsonRequest("PUT", JSON.stringify(request)),
        parseProductPreferences,
      );
    },

    async migrateM1BrowserState(exact, options) {
      const body = canonicalM1MigrationBody(exact);
      return requestJson(
        fetchImpl,
        productUrl(apiPrefix, "/product/migrations/m1-browser"),
        jsonRequest("POST", body, options?.signal),
        parseM1BrowserMigrationResponse,
      );
    },
  };
}
