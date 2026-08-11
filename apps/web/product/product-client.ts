import {
  ProductApiSchemaError,
  parseApiErrorResponse,
  parseCreateProductControlRequest,
  parseCreateProductForkRequest,
  parseCreateProductSessionRequest,
  parseCreateProductWorkspaceRequest,
  parseM1BrowserMigrationRequest,
  parseM1BrowserMigrationResponse,
  parseProductPreferences,
  parseProductControl,
  parseProductControlsResponse,
  parseProductForkResponse,
  parseProductForksResponse,
  parseProductProviderProfile,
  parseProductProviderModelsResponse,
  parseProductProviderProfileRequest,
  parseProductProviderProfilesResponse,
  parseProductSession,
  parseProductSessionModelConfigResponse,
  parseProductSessionRunModelsResponse,
  parseProductSessionUsageResponse,
  parseProductFilesResponse,
  parseProductFileContentEnvelope,
  parseProductArtifactsResponse,
  parseProductArtifactContentEnvelope,
  parseProductSessionDiffResponse,
  parseProductSessionsResponse,
  parseProductTranscriptResponse,
  parseProductWorkspace,
  parseUpdateProductSessionModelConfigRequest,
  parseProductWorkspacesResponse,
  parseUpdateProductPreferencesRequest,
  parseUpdateProductSessionRequest,
  type CreateProductProviderProfileRequest,
  type CreateProductControlRequest,
  type CreateProductForkRequest,
  type CreateProductSessionRequest,
  type CreateProductWorkspaceRequest,
  type M1BrowserMigrationRequest,
  type M1BrowserMigrationResponse,
  type ProductPreferences,
  type ProductControl,
  type ProductControlsResponse,
  type ProductControlStatusFilter,
  type ProductForkResponse,
  type ProductForksResponse,
  type ProductProviderProfile,
  type ProductProviderModelsResponse,
  type ProductProviderProfilesResponse,
  type ProductSession,
  type ProductSessionModelConfig,
  type ProductSessionRunModelsResponse,
  type ProductSessionUsageResponse,
  type ProductSessionDiffResponse,
  type ProductArtifactsResponse,
  type ProductArtifactContentEnvelope,
  type ProductFileContentEnvelope,
  type ProductFilesResponse,
  type ProductSessionsResponse,
  type ProductTranscriptResponse,
  type ProductWorkspace,
  type ProductWorkspacesResponse,
  type UpdateProductPreferencesRequest,
  type UpdateProductProviderProfileRequest,
  type UpdateProductSessionModelConfigRequest,
  type UpdateProductSessionRequest,
} from "./product-api-types";
import {
  desktopTransport,
  withDesktopAuthorization,
} from "../platform/desktop-transport";

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
  apiToken?: string;
}

export interface ExactM1BrowserMigrationBody {
  request: M1BrowserMigrationRequest;
  body: string;
}

export interface ProductApiRequestOptions {
  signal?: AbortSignal;
}

export const PRODUCT_EXPORT_FORMATS = ["json", "html", "markdown"] as const;
export type ProductExportFormat = (typeof PRODUCT_EXPORT_FORMATS)[number];

export interface ProductSessionEvidenceDownload {
  filename: string;
  mediaType: string;
  content: Blob;
}

const MAX_BINARY_RESOURCE_BYTES = 64 * 1024 * 1024;

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
  getSessionModelConfig(sessionId: string): Promise<ProductSessionModelConfig>;
  updateSessionModelConfig(
    sessionId: string,
    request: UpdateProductSessionModelConfigRequest,
  ): Promise<ProductSessionModelConfig>;
  listSessionRunModels(sessionId: string): Promise<ProductSessionRunModelsResponse>;
  getSessionUsage(sessionId: string): Promise<ProductSessionUsageResponse>;
  listWorkspaceFiles(workspaceId: string, query?: { prefix?: string; cursor?: string; limit?: number }): Promise<ProductFilesResponse>;
  getWorkspaceFileContent(workspaceId: string, path: string): Promise<ProductFileContentEnvelope>;
  workspaceFileDownloadUrl(workspaceId: string, path: string): string;
  workspaceFilePreviewUrl(workspaceId: string, path: string): string;
  fetchWorkspaceFileDownload(workspaceId: string, path: string): Promise<Blob>;
  fetchWorkspaceFilePreview(workspaceId: string, path: string): Promise<Blob>;
  listSessionArtifacts(sessionId: string, includeSystem?: boolean): Promise<ProductArtifactsResponse>;
  getArtifactContent(sessionId: string, artifactId: string): Promise<ProductArtifactContentEnvelope>;
  artifactDownloadUrl(sessionId: string, artifactId: string): string;
  artifactPreviewUrl(sessionId: string, artifactId: string): string;
  fetchArtifactDownload(sessionId: string, artifactId: string): Promise<Blob>;
  fetchArtifactPreview(sessionId: string, artifactId: string): Promise<Blob>;
  getSessionDiff(sessionId: string, scope?: "run" | "git" | "all"): Promise<ProductSessionDiffResponse>;
  exportSessionEvidence(
    sessionId: string,
    format: ProductExportFormat,
  ): Promise<ProductSessionEvidenceDownload>;
  deleteSession(sessionId: string): Promise<void>;
  createFork(
    sessionId: string,
    request: CreateProductForkRequest,
  ): Promise<ProductForkResponse>;
  listForks(sessionId: string): Promise<ProductForksResponse>;
  getTranscript(sessionId: string): Promise<ProductTranscriptResponse>;
  enqueueSteer(
    sessionId: string,
    request: CreateProductControlRequest,
  ): Promise<ProductControl>;
  enqueueFollowup(
    sessionId: string,
    request: CreateProductControlRequest,
  ): Promise<ProductControl>;
  listControls(
    sessionId: string,
    filter?: ProductControlStatusFilter,
  ): Promise<ProductControlsResponse>;
  revokeControl(sessionId: string, controlId: string): Promise<ProductControl>;
  confirmFollowup(sessionId: string, controlId: string): Promise<ProductControl>;
  listProviderProfiles(): Promise<ProductProviderProfilesResponse>;
  createProviderProfile(
    request: CreateProductProviderProfileRequest,
  ): Promise<ProductProviderProfile>;
  updateProviderProfile(
    profileId: string,
    request: UpdateProductProviderProfileRequest,
  ): Promise<ProductProviderProfile>;
  deleteProviderProfile(profileId: string): Promise<void>;
  listProviderModels(profileId: string): Promise<ProductProviderModelsResponse>;
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

async function requestEvidenceExport(
  fetchImpl: typeof globalThis.fetch,
  url: string,
  sessionId: string,
  format: ProductExportFormat,
): Promise<ProductSessionEvidenceDownload> {
  const response = await fetchImpl(url, { method: "POST", cache: "no-store" });
  if (!response.ok) {
    return throwProductApiError(response);
  }
  const expectedMediaType = {
    json: "application/json",
    html: "text/html",
    markdown: "text/markdown",
  }[format];
  const mediaType = response.headers.get("content-type") ?? "";
  if (!mediaType.toLowerCase().startsWith(expectedMediaType)) {
    throw new ProductApiSchemaError(
      `product evidence export must use ${expectedMediaType}`,
    );
  }
  const content = await response.blob();
  if (content.size > 16 * 1024 * 1024) {
    throw new ProductApiSchemaError("product evidence export exceeds the 16 MiB client limit");
  }
  const extension = format === "markdown" ? "md" : format;
  const fallback = `rove-session-${safeFilenamePart(sessionId)}-evidence.${extension}`;
  const filename = attachmentFilename(response.headers.get("content-disposition")) ?? fallback;
  return { filename, mediaType, content };
}

async function requestBinaryResource(
  fetchImpl: typeof globalThis.fetch,
  url: string,
  expectedMediaType?: string,
): Promise<Blob> {
  const response = await fetchImpl(url, { cache: "no-store" });
  if (!response.ok) {
    return throwProductApiError(response);
  }
  const mediaType = response.headers.get("content-type") ?? "";
  if (expectedMediaType && !mediaType.toLowerCase().startsWith(expectedMediaType)) {
    throw new ProductApiSchemaError(
      `product binary response must use ${expectedMediaType}`,
    );
  }
  const content = await response.blob();
  if (content.size > MAX_BINARY_RESOURCE_BYTES) {
    throw new ProductApiSchemaError(
      `product binary response exceeds the ${MAX_BINARY_RESOURCE_BYTES} byte client limit`,
    );
  }
  return content;
}

function attachmentFilename(value: string | null): string | null {
  const match = value?.match(/(?:^|;)\s*filename="([A-Za-z0-9._-]+)"\s*(?:;|$)/i);
  return match?.[1] ?? null;
}

function safeFilenamePart(value: string): string {
  const safe = value.replace(/[^A-Za-z0-9_-]/g, "-").replace(/-+/g, "-").slice(0, 96);
  return safe || "session";
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
  const baseFetch = options.fetch ?? globalThis.fetch;
  if (!baseFetch) {
    throw new Error("fetch is required to create a product API client");
  }
  const desktop = desktopTransport();
  const fetchImpl = withDesktopAuthorization(
    baseFetch,
    options.apiToken ?? desktop?.token,
  );
  const apiPrefix = normalizeApiPrefix(
    options.apiPrefix ?? desktop?.apiPrefix ?? DEFAULT_API_PREFIX,
  );

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

    async getSessionModelConfig(sessionId) {
      return requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/sessions/${encodeURIComponent(sessionId)}/model-config`,
        ),
        undefined,
        parseProductSessionModelConfigResponse,
      );
    },

    async updateSessionModelConfig(sessionId, input) {
      const request = parseUpdateProductSessionModelConfigRequest(input);
      return requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/sessions/${encodeURIComponent(sessionId)}/model-config`,
        ),
        jsonRequest("PUT", JSON.stringify(request)),
        parseProductSessionModelConfigResponse,
      );
    },

    listSessionRunModels(sessionId) {
      return requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/sessions/${encodeURIComponent(sessionId)}/run-models`,
        ),
        undefined,
        parseProductSessionRunModelsResponse,
      );
    },

    
    listWorkspaceFiles(workspaceId, query) {
      const params = new URLSearchParams();
      if (query?.prefix) params.set("prefix", query.prefix);
      if (query?.cursor) params.set("cursor", query.cursor);
      if (query?.limit !== undefined) params.set("limit", String(query.limit));
      const suffix = params.size ? `?${params.toString()}` : "";
      return requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/workspaces/${encodeURIComponent(workspaceId)}/files${suffix}`,
        ),
        undefined,
        parseProductFilesResponse,
      );
    },

    getWorkspaceFileContent(workspaceId, path) {
      const params = new URLSearchParams({ path });
      return requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/workspaces/${encodeURIComponent(workspaceId)}/files/content?${params.toString()}`,
        ),
        undefined,
        parseProductFileContentEnvelope,
      );
    },

    workspaceFileDownloadUrl(workspaceId, path) {
      const params = new URLSearchParams({ path });
      return productUrl(
        apiPrefix,
        `/product/workspaces/${encodeURIComponent(workspaceId)}/files/download?${params.toString()}`,
      );
    },

    workspaceFilePreviewUrl(workspaceId, path) {
      const params = new URLSearchParams({ path });
      return productUrl(
        apiPrefix,
        `/product/workspaces/${encodeURIComponent(workspaceId)}/files/preview?${params.toString()}`,
      );
    },

    fetchWorkspaceFileDownload(workspaceId, path) {
      return requestBinaryResource(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/workspaces/${encodeURIComponent(workspaceId)}/files/download?${new URLSearchParams({ path }).toString()}`,
        ),
      );
    },

    fetchWorkspaceFilePreview(workspaceId, path) {
      return requestBinaryResource(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/workspaces/${encodeURIComponent(workspaceId)}/files/preview?${new URLSearchParams({ path }).toString()}`,
        ),
        "image/",
      );
    },

    listSessionArtifacts(sessionId, includeSystem) {
      const params = new URLSearchParams();
      if (includeSystem !== undefined) {
        params.set("include_system", includeSystem ? "true" : "false");
      }
      const suffix = params.size ? `?${params.toString()}` : "";
      return requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/sessions/${encodeURIComponent(sessionId)}/artifacts${suffix}`,
        ),
        undefined,
        parseProductArtifactsResponse,
      );
    },

    getArtifactContent(sessionId, artifactId) {
      return requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/sessions/${encodeURIComponent(sessionId)}/artifacts/${encodeURIComponent(artifactId)}/content`,
        ),
        undefined,
        parseProductArtifactContentEnvelope,
      );
    },

    artifactDownloadUrl(sessionId, artifactId) {
      return productUrl(
        apiPrefix,
        `/product/sessions/${encodeURIComponent(sessionId)}/artifacts/${encodeURIComponent(artifactId)}/download`,
      );
    },

    artifactPreviewUrl(sessionId, artifactId) {
      return productUrl(
        apiPrefix,
        `/product/sessions/${encodeURIComponent(sessionId)}/artifacts/${encodeURIComponent(artifactId)}/preview`,
      );
    },

    fetchArtifactDownload(sessionId, artifactId) {
      return requestBinaryResource(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/sessions/${encodeURIComponent(sessionId)}/artifacts/${encodeURIComponent(artifactId)}/download`,
        ),
      );
    },

    fetchArtifactPreview(sessionId, artifactId) {
      return requestBinaryResource(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/sessions/${encodeURIComponent(sessionId)}/artifacts/${encodeURIComponent(artifactId)}/preview`,
        ),
        "image/",
      );
    },

    getSessionDiff(sessionId, scope) {
      const params = new URLSearchParams();
      if (scope) params.set("scope", scope);
      const suffix = params.size ? `?${params.toString()}` : "";
      return requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/sessions/${encodeURIComponent(sessionId)}/diff${suffix}`,
        ),
        undefined,
        parseProductSessionDiffResponse,
      );
    },

    exportSessionEvidence(sessionId, format) {
      if (!PRODUCT_EXPORT_FORMATS.includes(format)) {
        throw new ProductApiSchemaError("unsupported product evidence export format");
      }
      const query = new URLSearchParams({ format });
      return requestEvidenceExport(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/sessions/${encodeURIComponent(sessionId)}/export?${query.toString()}`,
        ),
        sessionId,
        format,
      );
    },

    getSessionUsage(sessionId) {
      return requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/sessions/${encodeURIComponent(sessionId)}/usage`,
        ),
        undefined,
        parseProductSessionUsageResponse,
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

    async createFork(sessionId, input) {
      const request = parseCreateProductForkRequest(input);
      return requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/sessions/${encodeURIComponent(sessionId)}/forks`,
        ),
        jsonRequest("POST", JSON.stringify(request)),
        parseProductForkResponse,
      );
    },

    listForks(sessionId) {
      return requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/sessions/${encodeURIComponent(sessionId)}/forks`,
        ),
        undefined,
        parseProductForksResponse,
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

    async enqueueSteer(sessionId, input) {
      return createProductControl(
        fetchImpl,
        apiPrefix,
        sessionId,
        "steers",
        input,
      );
    },

    async enqueueFollowup(sessionId, input) {
      return createProductControl(
        fetchImpl,
        apiPrefix,
        sessionId,
        "followups",
        input,
      );
    },

    listControls(sessionId, filter) {
      const query = filter ? `?status=${encodeURIComponent(filter)}` : "";
      return requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/sessions/${encodeURIComponent(sessionId)}/controls${query}`,
        ),
        undefined,
        parseProductControlsResponse,
      );
    },

    revokeControl(sessionId, controlId) {
      return requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/sessions/${encodeURIComponent(sessionId)}/controls/${encodeURIComponent(controlId)}/revoke`,
        ),
        jsonRequest("POST", "{}"),
        parseProductControl,
      );
    },

    confirmFollowup(sessionId, controlId) {
      return requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/sessions/${encodeURIComponent(sessionId)}/controls/${encodeURIComponent(controlId)}/confirm`,
        ),
        jsonRequest("POST", "{}"),
        parseProductControl,
      );
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

    listProviderModels(profileId) {
      return requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/provider-profiles/${encodeURIComponent(profileId)}/models`,
        ),
        undefined,
        parseProductProviderModelsResponse,
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

async function createProductControl(
  fetchImpl: typeof globalThis.fetch,
  apiPrefix: string,
  sessionId: string,
  endpoint: "steers" | "followups",
  input: CreateProductControlRequest,
): Promise<ProductControl> {
  const request = parseCreateProductControlRequest(input);
  return requestJson(
    fetchImpl,
    productUrl(
      apiPrefix,
      `/product/sessions/${encodeURIComponent(sessionId)}/${endpoint}`,
    ),
    jsonRequest("POST", JSON.stringify(request)),
    parseProductControl,
  );
}
