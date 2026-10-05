import {
  MAX_PRODUCT_TRANSCRIPT_PAGE_RUNS,
  MAX_PENDING_PRODUCT_MESSAGES,
  MAX_PRODUCT_SEARCH_LIMIT,
  MAX_PRODUCT_SEARCH_QUERY_BYTES,
  ProductApiSchemaError,
  parseApiErrorResponse,
  parseCreateProductControlRequest,
  parseCreateProductForkRequest,
  parseCreateProductReviewRequest,
  parseCreateProductSessionRequest,
  parseCreateProductWorkspaceRequest,
  parseM1BrowserMigrationRequest,
  parseM1BrowserMigrationResponse,
  parseProductAttachmentUpload,
  parseProductPreferences,
  parseProductControl,
  parseProductControlsResponse,
  parseProductMessage,
  parseProductMessagesResponse,
  parseProductMessagesSearchResponse,
  parseProductQueueResponse,
  parseProductSearchResponse,
  parseProductSessionCompaction,
  parseProductForkResponse,
  parseProductForksResponse,
  parseProductProviderProfile,
  parseProductProviderModelsResponse,
  parseProductProviderProfileRequest,
  parseProductProviderProfilesResponse,
  parseProductReview,
  parseProductReviewFindingsResponse,
  parseProductReviewsResponse,
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
  parseProductAuthorizationsResponse,
  parseCreateProductPreviewRequest,
  parseProductPreviewSession,
  parseProductWorkspace,
  parsePromoteProductMessageRequest,
  parseReorderProductMessagesRequest,
  parseUpdateProductSessionModelConfigRequest,
  parseProductWorkspacesResponse,
  parseUpdateProductPreferencesRequest,
  parseUpdateProductSessionRequest,
  type CreateProductProviderProfileRequest,
  type CreateProductControlRequest,
  type CreateProductForkRequest,
  type CreateProductReviewRequest,
  type CreateProductSessionRequest,
  type CreateProductWorkspaceRequest,
  type M1BrowserMigrationRequest,
  type M1BrowserMigrationResponse,
  type ProductPreferences,
  type ProductControl,
  type ProductControlsResponse,
  type ProductControlStatusFilter,
  type CreateProductMessageRequest,
  type ProductMessage,
  type ProductMessagesResponse,
  type ProductMessagesSearchResponse,
  type ProductQueueResponse,
  type ProductSearchResponse,
  type ProductSessionCompaction,
  type ProductForkResponse,
  type ProductForksResponse,
  type ProductProviderProfile,
  type ProductProviderModelsResponse,
  type ProductProviderProfilesResponse,
  type ProductReview,
  type ProductReviewFindingsResponse,
  type ProductReviewsResponse,
  type ProductSession,
  type ProductSessionModelConfig,
  type ProductSessionRunModelsResponse,
  type ProductSessionUsageResponse,
  type ProductSessionDiffResponse,
  type ProductAuthorizationsResponse,
  type CreateProductPreviewRequest,
  type ProductPreviewSession,
  type ProductArtifactsResponse,
  type ProductArtifactContentEnvelope,
  type ProductAttachmentUpload,
  type ProductFileContentEnvelope,
  type ProductFilesResponse,
  type ProductSessionsResponse,
  type ProductTranscriptResponse,
  type ProductWorkspace,
  type ProductWorkspacesResponse,
  type PromoteProductMessageRequest,
  type ReorderProductMessagesRequest,
  type UpdateProductPreferencesRequest,
  type UpdateProductProviderProfileRequest,
  type UpdateProductSessionModelConfigRequest,
  type UpdateProductSessionRequest,
} from "./product-api-types";
import { STREAM_EVENT_CONTRACT_VERSION } from "../lib/rove-types";
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

/**
 * The attachment download route's typed refusals, by status. A `HEAD` probe
 * carries no body, so the code the `GET` body would have carried is restored
 * from the status instead of from a payload.
 */
const PROBE_ATTACHMENT_STATUS_CODES: Record<number, string> = {
  404: "product_attachment_not_found",
  409: "product_attachment_conflict",
  410: "product_attachment_unavailable",
  429: "product_attachment_busy",
};

/** The claim an upload makes for a name the server's allow-list accepts. */
const CLAIM_CONTENT_TYPES: Record<string, string> = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  webp: "image/webp",
  gif: "image/gif",
  pdf: "application/pdf",
  txt: "text/plain",
  md: "text/markdown",
};

function claimContentType(name: string, declared: string): string {
  const dot = name.lastIndexOf(".");
  const extension = dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
  return CLAIM_CONTENT_TYPES[extension] ?? (declared.trim() || "application/octet-stream");
}

export interface ProductApiClient {
  listWorkspaces(): Promise<ProductWorkspacesResponse>;
  createWorkspace(
    request: CreateProductWorkspaceRequest,
  ): Promise<ProductWorkspace>;
  deleteWorkspace(workspaceId: string): Promise<void>;
  /** Opens the OS folder dialog via the local API and returns the absolute path. */
  pickWorkspaceFolder(): Promise<
    | { status: "selected"; path: string }
    | { status: "canceled" }
    | { status: "unavailable"; reason: string }
  >;
  listSessions(
    workspaceId: string,
    query?: {
      cursor?: string;
      limit?: number;
      q?: string;
      includeArchived?: boolean;
    },
  ): Promise<ProductSessionsResponse>;
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
  /**
   * Upload one file as a raw request body (R8). `fetch` cannot report
   * bytes-uploaded progress for a request in a portable way, so the caller
   * renders an indeterminate "uploading" row and a separate "validating" state
   * while this promise is unsettled — never a synthetic 100%. `name` is display
   * metadata and is never a path.
   */
  uploadAttachment(
    sessionId: string,
    file: Blob,
    options?: { name?: string; signal?: AbortSignal },
  ): Promise<ProductAttachmentUpload>;
  /**
   * One attachment's verified bytes. Attachments are fetched rather than
   * linked because a bare URL cannot carry the Desktop bearer token, which is
   * the same reason file and artifact previews are fetched into object URLs.
   */
  fetchAttachment(sessionId: string, attachmentId: string): Promise<Blob>;
  /**
   * Ask whether a reference is still readable without downloading it. The
   * download route answers `HEAD` too, and a probe carries no body, so the
   * status is the whole answer.
   */
  probeAttachment(
    sessionId: string,
    attachmentId: string,
    options?: ProductApiRequestOptions,
  ): Promise<void>;
  getSessionDiff(sessionId: string, scope?: "run" | "git" | "all"): Promise<ProductSessionDiffResponse>;
  getSessionAuthorizations(
    sessionId: string,
    query?: { limit?: number },
  ): Promise<ProductAuthorizationsResponse>;
  createWorkspacePreview(
    workspaceId: string,
    request: CreateProductPreviewRequest,
  ): Promise<ProductPreviewSession>;
  closeWorkspacePreview(
    workspaceId: string,
    previewId: string,
  ): Promise<void>;
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
  createReview(
    sessionId: string,
    request: CreateProductReviewRequest,
  ): Promise<ProductReview>;
  listReviews(sessionId: string): Promise<ProductReviewsResponse>;
  getReview(reviewId: string): Promise<ProductReview>;
  listReviewFindings(
    reviewId: string,
    query?: { cursor?: number; limit?: number },
  ): Promise<ProductReviewFindingsResponse>;
  cancelReview(reviewId: string): Promise<ProductReview>;
  getTranscript(
    sessionId: string,
    query?: { beforeOrdinal?: number; limitRuns?: number },
  ): Promise<ProductTranscriptResponse>;
  sendMessage(sessionId: string, request: CreateProductMessageRequest): Promise<ProductMessage>;
  listMessages(
    sessionId: string,
    query?: { afterSeq?: number; beforeSeq?: number; limit?: number },
  ): Promise<ProductMessagesResponse>;
  /** R7 step 1: bounded search over this session's message ledger. */
  searchSessionMessages(
    sessionId: string,
    query: { q: string; cursor?: string; limit?: number },
  ): Promise<ProductMessagesSearchResponse>;
  /** R7 step 2: unified search across a `workspace:` / `session:` / `trace:` scope. */
  searchProduct(query: {
    q: string;
    scope: string;
    cursor?: string;
    limit?: number;
  }): Promise<ProductSearchResponse>;
  /**
   * Promote one queued message. `delivery: "successor"` moves it to the head of
   * the successor queue and waits for the current turn's terminal boundary;
   * `current_run` (the server default when the body is empty) steers the live
   * turn instead.
   */
  promoteMessage(
    sessionId: string,
    messageId: string,
    request?: PromoteProductMessageRequest,
  ): Promise<ProductMessage>;
  /**
   * R4: rewrite the successor queue in one transaction. `ordered_ids` must name
   * exactly the currently queued successor messages; a stale list is refused
   * with a typed conflict rather than merged.
   */
  reorderMessages(
    sessionId: string,
    request: ReorderProductMessagesRequest,
  ): Promise<ProductQueueResponse>;
  /** R9: run the manual compaction the CLI's `/compact` runs. Idle sessions only. */
  compactSession(sessionId: string): Promise<ProductSessionCompaction>;
  revokeMessage(sessionId: string, messageId: string): Promise<ProductMessage>;
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
  deleteProviderProfile(profileId: string, expectedRevision?: string): Promise<void>;
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

/**
 * Cursor query for a transcript page. An unbounded or zero cursor is a caller
 * bug the server would answer with 400, so it fails before leaving the client.
 *
 * The request also declares the canonical-event contract this bundle can decode,
 * so a server that knows newer event kinds withholds those rows and reports their
 * durable positions rather than failing the restore.
 */
function transcriptPageQuery(
  query?: { beforeOrdinal?: number; limitRuns?: number },
): string {
  const params = new URLSearchParams();
  params.set("event_contract", String(STREAM_EVENT_CONTRACT_VERSION));
  if (query?.beforeOrdinal !== undefined) {
    if (
      !Number.isSafeInteger(query.beforeOrdinal) ||
      query.beforeOrdinal < 1
    ) {
      throw new RangeError("transcript before_ordinal must be a positive integer");
    }
    params.set("before_ordinal", String(query.beforeOrdinal));
  }
  if (query?.limitRuns !== undefined) {
    if (
      !Number.isSafeInteger(query.limitRuns) ||
      query.limitRuns < 1 ||
      query.limitRuns > MAX_PRODUCT_TRANSCRIPT_PAGE_RUNS
    ) {
      throw new RangeError(
        `transcript limit_runs must be between 1 and ${MAX_PRODUCT_TRANSCRIPT_PAGE_RUNS}`,
      );
    }
    params.set("limit_runs", String(query.limitRuns));
  }
  const encoded = params.toString();
  return encoded ? `?${encoded}` : "";
}

/**
 * Shared query for the two search endpoints.
 *
 * A term the server would refuse (blank, over the byte cap, or carrying a
 * control character) fails here instead of leaving the client, and a paginated
 * request can only reuse a cursor with the same term — the server binds the
 * cursor to the term and answers a mismatch with 400. The pairing rule lives in
 * one place so neither caller can drift from it.
 */
function searchPageQuery(query: {
  q: string;
  cursor?: string;
  limit?: number;
}): string {
  const term = query.q ?? "";
  const bytes = new TextEncoder().encode(term).length;
  if (term.trim().length === 0) {
    throw new RangeError("search q must not be blank");
  }
  if (bytes > MAX_PRODUCT_SEARCH_QUERY_BYTES) {
    throw new RangeError(
      `search q must be at most ${MAX_PRODUCT_SEARCH_QUERY_BYTES} UTF-8 bytes`,
    );
  }
  if (/[\u0000-\u001f\u007f-\u009f]/u.test(term)) {
    throw new RangeError("search q must be free of control characters");
  }
  const params = new URLSearchParams({ q: term });
  if (query.cursor !== undefined) {
    if (!query.cursor) {
      throw new RangeError("search cursor must not be empty");
    }
    params.set("cursor", query.cursor);
  }
  if (query.limit !== undefined) {
    if (
      !Number.isSafeInteger(query.limit) ||
      query.limit < 1 ||
      query.limit > MAX_PRODUCT_SEARCH_LIMIT
    ) {
      throw new RangeError(
        `search limit must be between 1 and ${MAX_PRODUCT_SEARCH_LIMIT}`,
      );
    }
    params.set("limit", String(query.limit));
  }
  return `?${params.toString()}`;
}

/** The three scopes the unified search resolves: `keyword:ulid`. */
const PRODUCT_SEARCH_SCOPE_PREFIXES = ["workspace", "session", "trace"] as const;

/**
 * Validate a scope before the request leaves. The server echoes the canonical
 * scope back, so the UI keys its state on the response rather than on what it
 * typed.
 */
export function expectProductSearchScope(scope: string): string {
  const separator = scope.indexOf(":");
  const prefix = separator < 0 ? "" : scope.slice(0, separator);
  const id = separator < 0 ? "" : scope.slice(separator + 1);
  if (
    !(PRODUCT_SEARCH_SCOPE_PREFIXES as readonly string[]).includes(prefix) ||
    !id ||
    id.length > 64 ||
    /[\u0000-\u001f\u007f-\u009f:\s]/u.test(id)
  ) {
    throw new RangeError(
      "search scope must be workspace:<id>, session:<id>, or trace:<id>",
    );
  }
  return `${prefix}:${id}`;
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

    async pickWorkspaceFolder() {
      const raw = await requestJson(
        fetchImpl,
        productUrl(apiPrefix, "/product/workspace-picker"),
        { method: "POST" },
        (value) => value,
      );
      const unavailable = {
        status: "unavailable" as const,
        reason: "native_folder_picker_unavailable",
      };
      if (typeof raw !== "object" || raw === null || Array.isArray(raw)) {
        return unavailable;
      }
      if (!("status" in raw)) {
        return unavailable;
      }
      switch (raw.status) {
        case "selected":
          return "path" in raw && typeof raw.path === "string" && raw.path.trim()
            ? { status: "selected" as const, path: raw.path }
            : unavailable;
        case "canceled":
          return { status: "canceled" as const };
        case "unavailable":
          return {
            ...unavailable,
            reason:
              "reason" in raw && typeof raw.reason === "string" && raw.reason.trim()
                ? raw.reason
                : unavailable.reason,
          };
        default:
          return unavailable;
      }
    },

    listSessions(workspaceId, query) {
      const params = new URLSearchParams({ workspace_id: workspaceId });
      if (query?.cursor) params.set("cursor", query.cursor);
      if (query?.limit !== undefined) params.set("limit", String(query.limit));
      if (query?.q) params.set("q", query.q);
      if (query?.includeArchived !== undefined) {
        params.set("include_archived", String(query.includeArchived));
      }
      return requestJson(
        fetchImpl,
        productUrl(apiPrefix, `/product/sessions?${params.toString()}`),
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

    async uploadAttachment(sessionId, file, options) {
      const name = options?.name?.trim();
      const params = new URLSearchParams();
      if (name) {
        params.set("name", name);
      }
      const encoded = params.toString();
      const response = await fetchImpl(
        productUrl(
          apiPrefix,
          `/product/sessions/${encodeURIComponent(sessionId)}/attachments${encoded ? `?${encoded}` : ""}`,
        ),
        {
          method: "POST",
          body: file,
          // The type is a claim the server checks against the bytes and never
          // stores unverified; naming it after the extension keeps that check
          // meaningful instead of echoing whatever the OS guessed.
          headers: {
            "content-type": claimContentType(name ?? "", file.type),
          },
          cache: "no-store",
          signal: options?.signal,
        },
      );
      if (!response.ok) {
        return throwProductApiError(response);
      }
      return parseProductAttachmentUpload(await readUnknownJson(response));
    },

    fetchAttachment(sessionId, attachmentId) {
      return requestBinaryResource(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/sessions/${encodeURIComponent(sessionId)}/attachments/${encodeURIComponent(attachmentId)}`,
        ),
      );
    },

    async probeAttachment(sessionId, attachmentId, options) {
      const response = await fetchImpl(
        productUrl(
          apiPrefix,
          `/product/sessions/${encodeURIComponent(sessionId)}/attachments/${encodeURIComponent(attachmentId)}`,
        ),
        { method: "HEAD", cache: "no-store", signal: options?.signal },
      );
      if (response.ok) {
        return;
      }
      // A probe has no body to parse, so the status is mapped onto the same
      // typed codes the download route's JSON body carries.
      throw new ProductApiError(
        response.status,
        PROBE_ATTACHMENT_STATUS_CODES[response.status] ?? "http_error",
        `attachment probe failed with status ${response.status}`,
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

    getSessionAuthorizations(sessionId, query) {
      const params = new URLSearchParams();
      if (query?.limit !== undefined) {
        params.set("limit", String(query.limit));
      }
      const encoded = params.toString();
      return requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/sessions/${encodeURIComponent(sessionId)}/authorizations${encoded ? `?${encoded}` : ""}`,
        ),
        undefined,
        parseProductAuthorizationsResponse,
      );
    },

    createWorkspacePreview(workspaceId, request) {
      const body = parseCreateProductPreviewRequest(request);
      return requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/workspaces/${encodeURIComponent(workspaceId)}/previews`,
        ),
        jsonRequest("POST", JSON.stringify(body)),
        parseProductPreviewSession,
      );
    },

    closeWorkspacePreview(workspaceId, previewId) {
      return requestNoContent(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/workspaces/${encodeURIComponent(workspaceId)}/previews/${encodeURIComponent(previewId)}`,
        ),
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

    async createReview(sessionId, input) {
      const request = parseCreateProductReviewRequest(input);
      return requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/sessions/${encodeURIComponent(sessionId)}/reviews`,
        ),
        jsonRequest("POST", JSON.stringify(request)),
        parseProductReview,
      );
    },

    listReviews(sessionId) {
      return requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/sessions/${encodeURIComponent(sessionId)}/reviews`,
        ),
        undefined,
        parseProductReviewsResponse,
      );
    },

    getReview(reviewId) {
      return requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/reviews/${encodeURIComponent(reviewId)}`,
        ),
        undefined,
        parseProductReview,
      );
    },

    listReviewFindings(reviewId, query) {
      const params = new URLSearchParams();
      if (query?.limit !== undefined) {
        params.set("limit", String(query.limit));
      }
      if (query?.cursor !== undefined) {
        params.set("cursor", String(query.cursor));
      }
      const suffix = params.size ? `?${params.toString()}` : "";
      return requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/reviews/${encodeURIComponent(reviewId)}/findings${suffix}`,
        ),
        undefined,
        parseProductReviewFindingsResponse,
      );
    },

    cancelReview(reviewId) {
      return requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/reviews/${encodeURIComponent(reviewId)}/cancel`,
        ),
        jsonRequest("POST", "{}"),
        parseProductReview,
      );
    },

    async getTranscript(sessionId, query) {
      const transcript = await requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/sessions/${encodeURIComponent(sessionId)}/transcript${transcriptPageQuery(query)}`,
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

    async sendMessage(sessionId, input) {
      const request = parseCreateProductControlRequest(input);
      return requestJson(
        fetchImpl,
        productUrl(apiPrefix, `/product/sessions/${encodeURIComponent(sessionId)}/messages`),
        jsonRequest("POST", JSON.stringify(request)),
        parseProductMessage,
      );
    },

    listMessages(sessionId, query) {
      const params = new URLSearchParams();
      if (query?.afterSeq !== undefined) {
        params.set("after_seq", String(query.afterSeq));
      }
      if (query?.beforeSeq !== undefined) {
        params.set("before_seq", String(query.beforeSeq));
      }
      if (query?.limit !== undefined) {
        params.set("limit", String(query.limit));
      }
      const encoded = params.toString();
      return requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/sessions/${encodeURIComponent(sessionId)}/messages${encoded ? `?${encoded}` : ""}`,
        ),
        undefined,
        parseProductMessagesResponse,
      );
    },

    promoteMessage(sessionId, messageId, request) {
      // The body stays exactly `{}` when the caller names no delivery, so the
      // legacy request is byte-identical to what it was before R4.
      const body =
        request === undefined
          ? "{}"
          : JSON.stringify(parsePromoteProductMessageRequest(request));
      return requestJson(
        fetchImpl,
        productUrl(apiPrefix, `/product/sessions/${encodeURIComponent(sessionId)}/messages/${encodeURIComponent(messageId)}/promote`),
        jsonRequest("POST", body),
        parseProductMessage,
      );
    },

    async reorderMessages(sessionId, input) {
      const request = parseReorderProductMessagesRequest(input);
      return requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/sessions/${encodeURIComponent(sessionId)}/messages/reorder`,
        ),
        jsonRequest("POST", JSON.stringify(request)),
        parseProductQueueResponse,
      );
    },

    async searchSessionMessages(sessionId, query) {
      // `searchPageQuery` refuses a term the server would answer with 400; it is
      // called inside the async body so the refusal reaches the caller as a
      // rejection like every other refusal in this client, not as a synchronous
      // throw from an argument list.
      const suffix = searchPageQuery(query);
      return requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/sessions/${encodeURIComponent(sessionId)}/search${suffix}`,
        ),
        undefined,
        parseProductMessagesSearchResponse,
      );
    },

    async searchProduct(query) {
      const scope = expectProductSearchScope(query.scope);
      const params = new URLSearchParams(searchPageQuery(query));
      params.set("scope", scope);
      return requestJson(
        fetchImpl,
        productUrl(apiPrefix, `/product/search?${params.toString()}`),
        undefined,
        parseProductSearchResponse,
      );
    },

    async compactSession(sessionId) {
      const compaction = await requestJson(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/sessions/${encodeURIComponent(sessionId)}/compact`,
        ),
        jsonRequest("POST", "{}"),
        parseProductSessionCompaction,
      );
      if (compaction.product_session_id !== sessionId) {
        throw new ProductApiSchemaError(
          "product compaction response must match the requested product session",
        );
      }
      return compaction;
    },

    revokeMessage(sessionId, messageId) {
      return requestJson(
        fetchImpl,
        productUrl(apiPrefix, `/product/sessions/${encodeURIComponent(sessionId)}/messages/${encodeURIComponent(messageId)}/revoke`),
        jsonRequest("POST", "{}"),
        parseProductMessage,
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

    deleteProviderProfile(profileId, expectedRevision) {
      const query = expectedRevision
        ? `?expected_revision=${encodeURIComponent(expectedRevision)}`
        : "";
      return requestNoContent(
        fetchImpl,
        productUrl(
          apiPrefix,
          `/product/provider-profiles/${encodeURIComponent(profileId)}${query}`,
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
