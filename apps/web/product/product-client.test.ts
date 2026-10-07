import { afterEach, describe, expect, it, vi } from "vitest";

import {
  MAX_PENDING_PRODUCT_MESSAGES,
  ProductApiSchemaError,
  PRODUCT_ERROR_CODES,
  isKnownProductCompactionMode,
  parseCreateProductControlRequest,
  parseCreateProductReviewRequest,
  parseProductControl,
  parseProductPreferences,
  parseProductProviderProfileRequest,
  parseProductSearchResponse,
  parseProductSessionCompaction,
  parseProductTranscriptResponse,
  parseUpdateProductPreferencesRequest,
  parseStreamEvent,
} from "./product-api-types";
import { createProductApiClient } from "./product-client";
import { STREAM_EVENT_CONTRACT_VERSION } from "../lib/rove-types";

const workspace = {
  id: "01J00000000000000000000001",
  canonical_root: "C:\\workspace\\demo",
  kind: "repo",
  display_name: "rove",
  pinned: true,
  last_opened_at: "2026-07-26T00:00:00.000Z",
  created_at: "2026-07-26T00:00:00.000Z",
  updated_at: "2026-07-26T00:00:00.000Z",
};

const session = {
  id: "01J00000000000000000000002",
  workspace_id: workspace.id,
  title: "Session",
  status: "idle",
  created_at: "2026-07-26T00:00:00.000Z",
  updated_at: "2026-07-26T00:00:00.000Z",
};

const forkedSession = {
  ...session,
  id: "01J00000000000000000000008",
  title: "Session fork",
  parent_session_id: session.id,
  fork_point_run_id: "01J00000000000000000000006",
  fork_point_seq: 4,
};

const fork = {
  id: "01J00000000000000000000009",
  parent_product_session_id: session.id,
  child_product_session_id: forkedSession.id,
  parent_workspace_id: workspace.id,
  parent_title: session.title,
  source_runtime_session_id: "01J00000000000000000000004",
  source_runtime_job_id: "01J00000000000000000000005",
  source_runtime_run_id: "01J00000000000000000000006",
  fork_at_event_seq: 4,
  idempotency_key: "fork-session-1",
  created_at: "2026-07-26T00:00:00.000Z",
};

const providerProfile = {
  id: "01J00000000000000000000003",
  label: "Gateway",
  provider_type: "openai",
  api_base: "https://gateway.example.test/v1",
  api_key_env: "GATEWAY_API_KEY",
  default_model: "test/model",
  created_at: "2026-07-26T00:00:00.000Z",
  updated_at: "2026-07-26T00:00:00.000Z",
  catalog_revision: "sha256:provider-catalog-1",
};

const sessionModelConfig = {
  product_session_id: session.id,
  profile_id: providerProfile.id,
  model: "test/model",
  reasoning: "default",
  max_steps: 8,
  revision: 1,
  updated_at: "2026-07-26T00:00:00.000Z",
};

const preferences = {
  schema_version: 1,
  revision: 3,
  theme: "dark",
  default_approval_policy: "ask",
  active_workspace_id: workspace.id,
  active_session_id: session.id,
  provider_selection: {
    profile_id: providerProfile.id,
    model: "test/model",
    approval: "ask",
    max_steps: 8,
  },
};

const control = {
  id: "01J00000000000000000000007",
  product_session_id: session.id,
  kind: "steer",
  idempotency_key: "control-1",
  content: "use the safe path",
  status: "pending",
  seq: 1,
  created_at: "2026-07-26T00:00:00.000Z",
};

const reviewFinding = {
  finding_id: "rfd_0123456789abcdef",
  severity: "high",
  confidence: "high",
  category: "correctness",
  path: "src/lib.rs",
  location: {
    start_line: 12,
    start_col: 1,
    end_line: 12,
    end_col: 8,
  },
  location_status: "validated",
  title: "Incorrect branch",
  explanation: "The changed branch returns the wrong value.",
  evidence: [{ snippet: "return false", source: "diff" }],
  rule: "correctness",
  suggestion: "Return the validated value.",
  status: "open",
};

const reviewTarget = {
  schema_version: 1,
  spec: { kind: "uncommitted" },
  workspace_kind: "repo",
  workspace_digest: "sha256:workspace",
  captured_at: "2026-08-17T00:00:00.000Z",
  entries: 1,
  entries_truncated: 0,
  digest: "sha256:target",
};

const review = {
  id: "01J00000000000000000000010",
  product_session_id: session.id,
  workspace_id: workspace.id,
  target: reviewTarget,
  status: "findings",
  conclusion: "findings",
  runtime_session_id: "01J00000000000000000000011",
  job_id: "01J00000000000000000000012",
  run_id: "01J00000000000000000000013",
  result: {
    schema_version: 1,
    review_id: "01J00000000000000000000010",
    run_id: "01J00000000000000000000013",
    session_id: "01J00000000000000000000011",
    target: reviewTarget,
    conclusion: "findings",
    findings: [reviewFinding],
    stats: {
      files_scanned: 1,
      bytes_scanned: 128,
      duration_ms: 10,
      concurrency_limit: 1,
      findings_total: 1,
      truncated_findings: 0,
    },
    unchecked: [],
    warnings: [],
  },
  findings_count: 1,
  unchecked_count: 0,
  warnings_count: 0,
  created_at: "2026-08-17T00:00:00.000Z",
  updated_at: "2026-08-17T00:00:01.000Z",
  captured_at: "2026-08-17T00:00:00.000Z",
  finalized_at: "2026-08-17T00:00:01.000Z",
};

function transcriptSegment(
  ordinal = 1,
  productSessionId = session.id,
): Record<string, unknown> {
  return {
    binding: {
      product_session_id: productSessionId,
      ordinal,
      runtime_session_id: "01J00000000000000000000004",
      runtime_job_id: "01J00000000000000000000005",
      runtime_run_id: `01J0000000000000000000000${5 + ordinal}`,
      bound_at: "2026-07-26T00:00:00.000Z",
    },
    inherited: false,
    run_status: "running",
    observed_through_seq: 1,
    last_event_seq: 1,
    events: [
      {
        seq: 1,
        event: {
          type: "run_started",
          run_id: `01J0000000000000000000000${5 + ordinal}`,
          job_id: "01J00000000000000000000005",
          user_message: "hello",
        },
      },
    ],
  };
}

function transcriptResponse(
  overrides: Record<string, unknown> = {},
): Record<string, unknown> {
  return {
    product_session_id: session.id,
    workspace_id: workspace.id,
    status: "complete",
    partial_reasons: [],
    segments: [transcriptSegment()],
    ...overrides,
  };
}

function jsonResponse(value: unknown, status = 200): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json" },
  });
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("product API client", () => {
  it.each([
    null, [], "selected", 42, false, {},
    { status: null }, { status: "unknown", reason: "untrusted" },
    { status: "selected" }, { status: "selected", path: "   " },
    { status: "selected", path: 42 },
  ].map((payload) => [payload]))("maps malformed picker payload %j to unavailable", async (payload) => {
    const client = createProductApiClient({ fetch: vi.fn(async () => jsonResponse(payload)) });
    await expect(client.pickWorkspaceFolder()).resolves.toEqual({
      status: "unavailable", reason: "native_folder_picker_unavailable",
    });
  });

  it.each([
    { status: "selected", path: "D:\\project" },
    { status: "canceled" },
    { status: "unavailable", reason: "unsupported_platform" },
  ])("preserves the validated picker response %j", async (payload) => {
    const fetchMock = vi.fn(async () => jsonResponse(payload));
    const client = createProductApiClient({ fetch: fetchMock });
    await expect(client.pickWorkspaceFolder()).resolves.toEqual(payload);
    expect(fetchMock).toHaveBeenCalledWith("/api/product/workspace-picker", { method: "POST" });
  });

  it("preserves typed JSON and HTTP picker errors", async () => {
    const malformed = createProductApiClient({ fetch: vi.fn(async () => new Response("not JSON")) });
    await expect(malformed.pickWorkspaceFolder()).rejects.toBeInstanceOf(ProductApiSchemaError);
    const failed = createProductApiClient({ fetch: vi.fn(async () => jsonResponse({ code: "unavailable", error: "Unavailable" }, 503)) });
    await expect(failed.pickWorkspaceFolder()).rejects.toMatchObject({ status: 503, code: "unavailable" });
  });

  it("uses the authenticated Desktop loopback transport", async () => {
    vi.stubGlobal("window", {
      __ROVE_API_URL__: "http://127.0.0.1:49152",
      __ROVE_TOKEN__: "desktop-secret",
    });
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, _init?: RequestInit) =>
        jsonResponse({ workspaces: [] }),
    );

    await createProductApiClient({ fetch: fetchMock }).listWorkspaces();

    expect(fetchMock).toHaveBeenCalledOnce();
    expect(String(fetchMock.mock.calls[0]?.[0])).toBe(
      "http://127.0.0.1:49152/product/workspaces",
    );
    const headers = new Headers(fetchMock.mock.calls[0]?.[1]?.headers);
    expect(headers.get("authorization")).toBe("Bearer desktop-secret");
  });

  it("authenticates Desktop binary resources instead of exposing a bare URL", async () => {
    vi.stubGlobal("window", {
      __ROVE_API_URL__: "http://127.0.0.1:49152",
      __ROVE_TOKEN__: "desktop-secret",
    });
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, _init?: RequestInit) =>
        new Response(new Uint8Array([1, 2, 3]), {
          status: 200,
          headers: { "content-type": "application/octet-stream" },
        }),
    );

    const blob = await createProductApiClient({ fetch: fetchMock }).fetchArtifactDownload(
      session.id,
      "artifact-1",
    );

    expect(blob.size).toBe(3);
    expect(String(fetchMock.mock.calls[0]?.[0])).toBe(
      `http://127.0.0.1:49152/product/sessions/${session.id}/artifacts/artifact-1/download`,
    );
    const headers = new Headers(fetchMock.mock.calls[0]?.[1]?.headers);
    expect(headers.get("authorization")).toBe("Bearer desktop-secret");
  });

  it("encodes bounded message pagination and validates continuation cursors", async () => {
    const message = {
      id: control.id,
      product_session_id: session.id,
      content: "continue in order",
      requested_delivery: "successor",
      status: "queued",
      seq: 41,
      created_at: "2026-08-14T00:00:00.000Z",
    };
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, _init?: RequestInit) =>
        jsonResponse({
          messages: [message],
          next_after_seq: 41,
          next_before_seq: 40,
        }),
    );
    const client = createProductApiClient({ fetch: fetchMock });

    const response = await client.listMessages(session.id, {
      afterSeq: 20,
      limit: 1,
    });

    expect(String(fetchMock.mock.calls[0]?.[0])).toBe(
      `/api/product/sessions/${session.id}/messages?after_seq=20&limit=1`,
    );
    expect(response.next_after_seq).toBe(41);
    expect(response.next_before_seq).toBe(40);
    expect(response.messages[0]?.content).toBe("continue in order");
  });

  it("covers the registered product CRUD, preferences, and transcript routes", async () => {
    const calls: Array<{ url: string; method: string; body?: string }> = [];
    const fetchMock: typeof globalThis.fetch = vi.fn(
      async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        const method = init?.method ?? "GET";
        calls.push({
          url,
          method,
          body: typeof init?.body === "string" ? init.body : undefined,
        });

        if (method === "DELETE") {
          return new Response(null, { status: 204 });
        }
        if (url === "/api/product/workspaces" && method === "GET") {
          return jsonResponse({ workspaces: [workspace] });
        }
        if (url === "/api/product/workspaces" && method === "POST") {
          return jsonResponse(workspace, 201);
        }
        if (url.startsWith("/api/product/sessions?")) {
          return jsonResponse({ sessions: [session] });
        }
        if (url === "/api/product/sessions" && method === "POST") {
          return jsonResponse(session, 201);
        }
        if (url.endsWith(`/product/sessions/${session.id}/forks`) && method === "POST") {
          return jsonResponse({ fork, session: forkedSession }, 201);
        }
        if (url.endsWith(`/product/sessions/${session.id}/forks`) && method === "GET") {
          return jsonResponse({ forks: [fork] });
        }
        if (url.endsWith(`/product/sessions/${session.id}/reviews`) && method === "POST") {
          return jsonResponse(review, 201);
        }
        if (url.endsWith(`/product/sessions/${session.id}/reviews`) && method === "GET") {
          return jsonResponse({ reviews: [review] });
        }
        if (url.endsWith(`/product/reviews/${review.id}/findings?limit=1&cursor=0`)) {
          return jsonResponse({
            findings: [{ finding: reviewFinding, sort_key: "001" }],
            next_cursor: 1,
          });
        }
        if (url.endsWith(`/product/reviews/${review.id}/cancel`)) {
          return jsonResponse({ ...review, status: "cancelled", conclusion: "cancelled" });
        }
        if (url.endsWith(`/product/reviews/${review.id}`)) {
          return jsonResponse(review);
        }
        if (url.endsWith(`/product/sessions/${session.id}`)) {
          return jsonResponse({ ...session, title: "Renamed" });
        }
        if (
          url.startsWith(`/api/product/sessions/${session.id}/transcript?`)
        ) {
          return jsonResponse({
            product_session_id: session.id,
            workspace_id: workspace.id,
            status: "complete",
            partial_reasons: [],
            segments: [],
          });
        }
        if (url.endsWith(`/product/sessions/${session.id}/model-config`)) {
          return jsonResponse(sessionModelConfig);
        }
        if (url.endsWith(`/product/sessions/${session.id}/run-models`)) {
          return jsonResponse({ runs: [] });
        }
        if (url.endsWith(`/product/sessions/${session.id}/steers`)) {
          return jsonResponse(control, 201);
        }
        if (url.endsWith(`/product/sessions/${session.id}/followups`)) {
          return jsonResponse({ ...control, kind: "followup", seq: 2 }, 201);
        }
        if (url.endsWith(`/product/sessions/${session.id}/controls`)) {
          return jsonResponse({ controls: [control] });
        }
        if (url.endsWith(`/controls/${control.id}/revoke`)) {
          return jsonResponse({ ...control, status: "revoked" });
        }
        if (url.endsWith(`/controls/${control.id}/confirm`)) {
          return jsonResponse({ ...control, kind: "followup", status: "pending" });
        }
        if (url === "/api/product/provider-profiles" && method === "GET") {
          return jsonResponse({
            catalog_revision: providerProfile.catalog_revision,
            provider_profiles: [providerProfile],
          });
        }
        if (url === "/api/product/provider-profiles" && method === "POST") {
          return jsonResponse(providerProfile, 201);
        }
        if (url.endsWith(`/product/provider-profiles/${providerProfile.id}`)) {
          return jsonResponse({ ...providerProfile, label: "Updated" });
        }
        if (url.endsWith(`/product/provider-profiles/${providerProfile.id}/models`)) {
          return jsonResponse({
            profile_id: providerProfile.id,
            default_model: providerProfile.default_model,
            models: [
              {
                id: providerProfile.default_model,
                supports_reasoning: false,
                supported_reasoning: [],
                reasoning_unavailable_reason:
                  "Reasoning controls are only available for OpenAI Responses profiles.",
              },
            ],
          });
        }
        if (url === "/api/product/preferences" && method === "GET") {
          return jsonResponse(preferences);
        }
        if (url === "/api/product/preferences" && method === "PUT") {
          return jsonResponse(preferences);
        }
        return jsonResponse({ code: "not_found", error: "unexpected route" }, 404);
      },
    );
    const client = createProductApiClient({ fetch: fetchMock });

    await client.listWorkspaces();
    await client.createWorkspace({
      root: workspace.canonical_root,
      kind: "repo",
      display_name: "rove",
      pinned: true,
    });
    await client.deleteWorkspace(workspace.id);
    await client.listSessions(workspace.id);
    await client.createSession({ workspace_id: workspace.id, title: "Session" });
    await client.createFork(session.id, {
      fork_at_run_id: fork.source_runtime_run_id,
      idempotency_key: fork.idempotency_key,
    });
    await client.listForks(session.id);
    await client.createReview(session.id, {
      target: { kind: "uncommitted" },
      idempotency_key: "review-1",
      max_steps: 8,
    });
    await client.listReviews(session.id);
    await client.getReview(review.id);
    await client.listReviewFindings(review.id, { limit: 1, cursor: 0 });
    await client.cancelReview(review.id);
    await client.updateSession(session.id, { title: "Renamed" });
    await client.getTranscript(session.id);
    await client.getSessionModelConfig(session.id);
    await client.updateSessionModelConfig(session.id, {
      profile_id: providerProfile.id,
      model: "test/model",
      reasoning: "default",
      max_steps: 8,
      expected_revision: 1,
    });
    await client.listSessionRunModels(session.id);
    await client.enqueueSteer(session.id, {
      content: control.content,
      idempotency_key: control.idempotency_key,
    });
    await client.enqueueFollowup(session.id, {
      content: "continue after the final answer",
      idempotency_key: "control-2",
    });
    await client.listControls(session.id);
    await client.revokeControl(session.id, control.id);
    await client.confirmFollowup(session.id, control.id);
    await client.deleteSession(session.id);
    await client.listProviderProfiles();
    await client.createProviderProfile({
      label: "Gateway",
      provider_type: "openai",
      api_base: "https://gateway.example.test/v1",
      api_key_env: "GATEWAY_API_KEY",
      default_model: "test/model",
      expected_revision: providerProfile.catalog_revision,
    });
    await client.updateProviderProfile(providerProfile.id, {
      label: "Updated",
      provider_type: "openai",
      api_base: "https://gateway.example.test/v1",
      api_key_env: "GATEWAY_API_KEY",
      default_model: "test/model",
      expected_revision: providerProfile.catalog_revision,
    });
    await client.listProviderModels(providerProfile.id);
    await client.deleteProviderProfile(
      providerProfile.id,
      providerProfile.catalog_revision,
    );
    await client.getPreferences();
    await client.updatePreferences({
      schema_version: 1,
      expected_revision: 3,
      theme: "dark",
      default_approval_policy: "ask",
      active_workspace_id: workspace.id,
      active_session_id: session.id,
      provider_selection: {
        profile_id: providerProfile.id,
        model: "test/model",
        approval: "ask",
        max_steps: 8,
      },
    });

    expect(calls.map(({ url, method }) => `${method} ${url}`)).toEqual([
      "GET /api/product/workspaces",
      "POST /api/product/workspaces",
      `DELETE /api/product/workspaces/${workspace.id}`,
      `GET /api/product/sessions?workspace_id=${workspace.id}`,
      "POST /api/product/sessions",
      `POST /api/product/sessions/${session.id}/forks`,
      `GET /api/product/sessions/${session.id}/forks`,
      `POST /api/product/sessions/${session.id}/reviews`,
      `GET /api/product/sessions/${session.id}/reviews`,
      `GET /api/product/reviews/${review.id}`,
      `GET /api/product/reviews/${review.id}/findings?limit=1&cursor=0`,
      `POST /api/product/reviews/${review.id}/cancel`,
      `PATCH /api/product/sessions/${session.id}`,
      `GET /api/product/sessions/${session.id}/transcript?event_contract=${STREAM_EVENT_CONTRACT_VERSION}`,
      `GET /api/product/sessions/${session.id}/model-config`,
      `PUT /api/product/sessions/${session.id}/model-config`,
      `GET /api/product/sessions/${session.id}/run-models`,
      `POST /api/product/sessions/${session.id}/steers`,
      `POST /api/product/sessions/${session.id}/followups`,
      `GET /api/product/sessions/${session.id}/controls`,
      `POST /api/product/sessions/${session.id}/controls/${control.id}/revoke`,
      `POST /api/product/sessions/${session.id}/controls/${control.id}/confirm`,
      `DELETE /api/product/sessions/${session.id}`,
      "GET /api/product/provider-profiles",
      "POST /api/product/provider-profiles",
      `PUT /api/product/provider-profiles/${providerProfile.id}`,
      `GET /api/product/provider-profiles/${providerProfile.id}/models`,
      `DELETE /api/product/provider-profiles/${providerProfile.id}?expected_revision=${encodeURIComponent(providerProfile.catalog_revision)}`,
      "GET /api/product/preferences",
      "PUT /api/product/preferences",
    ]);
  });

  it("rejects malformed successful responses instead of casting them", async () => {
    const client = createProductApiClient({
      fetch: vi.fn(async () =>
        jsonResponse({ workspaces: [{ ...workspace, pinned: "yes" }] }),
      ),
    });

    await expect(client.listWorkspaces()).rejects.toBeInstanceOf(
      ProductApiSchemaError,
    );
  });

  it("validates Review targets before dispatch", () => {
    expect(
      parseCreateProductReviewRequest({
        target: { kind: "base", revision: "main" },
        idempotency_key: "review-main-1",
        max_steps: 12,
      }),
    ).toEqual({
      target: { kind: "base", revision: "main" },
      idempotency_key: "review-main-1",
      max_steps: 12,
    });
    expect(() =>
      parseCreateProductReviewRequest({
        target: { kind: "uncommitted", revision: "HEAD" },
      }),
    ).toThrow(ProductApiSchemaError);
    expect(() =>
      parseCreateProductReviewRequest({ target: { kind: "commit" } }),
    ).toThrow(ProductApiSchemaError);
  });

  it("strictly validates control requests and responses before accepting them", () => {
    expect(
      parseCreateProductControlRequest({
        content: "  keep the migration safe ",
        idempotency_key: "control-1",
      }),
    ).toEqual({ content: "keep the migration safe", idempotency_key: "control-1" });
    expect(() =>
      parseCreateProductControlRequest({ content: "", unexpected: true }),
    ).toThrow(ProductApiSchemaError);
    expect(parseProductControl(control)).toEqual(control);
    expect(() => parseProductControl({ ...control, seq: 0 })).toThrow(
      ProductApiSchemaError,
    );
    expect(() => parseProductControl({ ...control, status: "unknown" })).toThrow(
      ProductApiSchemaError,
    );
  });

  it("surfaces typed control API failures instead of treating them as queued", async () => {
    const client = createProductApiClient({
      fetch: vi.fn(async () =>
        jsonResponse(
          { code: "product_control_conflict", error: "idempotency key differs" },
          409,
        ),
      ),
    });

    await expect(
      client.enqueueSteer(session.id, {
        content: "safe path",
        idempotency_key: "same-key",
      }),
    ).rejects.toMatchObject({
      status: 409,
      code: "product_control_conflict",
    });
  });

  it("parses revisioned preferences strictly while retaining legacy write compatibility", () => {
    expect(parseProductPreferences(preferences)).toEqual(preferences);
    expect(() =>
      parseProductPreferences({ ...preferences, revision: undefined }),
    ).toThrow(ProductApiSchemaError);
    expect(() =>
      parseProductPreferences({ ...preferences, unexpected: true }),
    ).toThrow(ProductApiSchemaError);

    expect(
      parseUpdateProductPreferencesRequest({
        schema_version: 1,
        theme: "system",
      }),
    ).toEqual({ schema_version: 1, theme: "system" });
    expect(
      parseUpdateProductPreferencesRequest({
        schema_version: 1,
        expected_revision: 3,
        theme: "dark",
        default_approval_policy: "never",
      }),
    ).toEqual({
      schema_version: 1,
      expected_revision: 3,
      theme: "dark",
      default_approval_policy: "never",
    });
    expect(PRODUCT_ERROR_CODES).toEqual(
      expect.arrayContaining([
        "product_revision_conflict",
        "product_memory_invalid_slug",
        "product_memory_not_found",
        "product_memory_conflict",
      ]),
    );
  });

  it("rejects a transcript for a different requested product session", async () => {
    const client = createProductApiClient({
      fetch: vi.fn(async () =>
        jsonResponse(
          transcriptResponse({
            product_session_id: "01J00000000000000000000099",
            segments: [],
          }),
        ),
      ),
    });

    await expect(client.getTranscript(session.id)).rejects.toBeInstanceOf(
      ProductApiSchemaError,
    );
  });

  it("rejects zero-based transcript run ordinals and event sequences", () => {
    const transcript = {
      product_session_id: session.id,
      workspace_id: workspace.id,
      status: "complete",
      partial_reasons: [],
      segments: [
        {
          binding: {
            product_session_id: session.id,
            ordinal: 1,
            runtime_session_id: "01J00000000000000000000004",
            runtime_job_id: "01J00000000000000000000005",
            runtime_run_id: "01J00000000000000000000006",
            bound_at: "2026-07-26T00:00:00.000Z",
          },
          run_status: "done",
          observed_through_seq: 1,
          last_event_seq: 1,
          events: [
            {
              seq: 0,
              event: {
                type: "run_started",
                run_id: "01J00000000000000000000006",
                job_id: "01J00000000000000000000005",
                user_message: "hello",
              },
            },
          ],
        },
      ],
    };

    expect(() => parseProductTranscriptResponse(transcript)).toThrow(
      ProductApiSchemaError,
    );
    transcript.segments[0]!.events[0]!.seq = 1;
    transcript.segments[0]!.binding.ordinal = 0;
    expect(() => parseProductTranscriptResponse(transcript)).toThrow(
      ProductApiSchemaError,
    );
  });

  it("rejects transcript segments bound to another product session", () => {
    expect(() =>
      parseProductTranscriptResponse(
        transcriptResponse({
          segments: [
            transcriptSegment(1, "01J00000000000000000000099"),
          ],
        }),
      ),
    ).toThrow(ProductApiSchemaError);
  });

  it("rejects duplicate or decreasing transcript segment ordinals", () => {
    expect(() =>
      parseProductTranscriptResponse(
        transcriptResponse({
          segments: [transcriptSegment(1), transcriptSegment(1)],
        }),
      ),
    ).toThrow(ProductApiSchemaError);

    expect(() =>
      parseProductTranscriptResponse(
        transcriptResponse({
          status: "partial",
          partial_reasons: [
            { code: "runtime_run_missing", run_ordinal: 1 },
          ],
          segments: [transcriptSegment(2), transcriptSegment(1)],
        }),
      ),
    ).toThrow(ProductApiSchemaError);
  });

  it("requires every transcript ordinal gap to have a typed partial reason", () => {
    expect(() =>
      parseProductTranscriptResponse(
        transcriptResponse({
          status: "partial",
          partial_reasons: [
            { code: "runtime_run_missing", run_ordinal: 3 },
          ],
          segments: [transcriptSegment(2)],
        }),
      ),
    ).toThrow(ProductApiSchemaError);

    expect(
      parseProductTranscriptResponse(
        transcriptResponse({
          status: "partial",
          partial_reasons: [
            { code: "runtime_run_missing", run_ordinal: 1 },
          ],
          segments: [transcriptSegment(2)],
        }),
      ).segments[0]?.binding.ordinal,
    ).toBe(2);
  });

  it("rejects non-contiguous transcript events and inconsistent watermarks", () => {
    const nonContiguous = transcriptSegment() as {
      events: Array<Record<string, unknown>>;
      observed_through_seq: number;
      last_event_seq: number;
    };
    nonContiguous.events.push({
      seq: 3,
      event: { type: "memory_flushed", notes: [] },
    });
    nonContiguous.observed_through_seq = 3;
    nonContiguous.last_event_seq = 3;
    expect(() =>
      parseProductTranscriptResponse(
        transcriptResponse({ segments: [nonContiguous] }),
      ),
    ).toThrow(ProductApiSchemaError);

    const mismatchedObserved = transcriptSegment() as {
      observed_through_seq: number;
    };
    mismatchedObserved.observed_through_seq = 0;
    expect(() =>
      parseProductTranscriptResponse(
        transcriptResponse({ segments: [mismatchedObserved] }),
      ),
    ).toThrow(ProductApiSchemaError);

    const beyondHighWater = transcriptSegment() as {
      last_event_seq: number;
    };
    beyondHighWater.last_event_seq = 0;
    expect(() =>
      parseProductTranscriptResponse(
        transcriptResponse({ segments: [beyondHighWater] }),
      ),
    ).toThrow(ProductApiSchemaError);

    const unexplainedMissingTail = transcriptSegment() as {
      last_event_seq: number;
    };
    unexplainedMissingTail.last_event_seq = 2;
    expect(() =>
      parseProductTranscriptResponse(
        transcriptResponse({ segments: [unexplainedMissingTail] }),
      ),
    ).toThrow(ProductApiSchemaError);

    expect(
      parseProductTranscriptResponse(
        transcriptResponse({
          status: "partial",
          partial_reasons: [
            {
              code: "missing_event_range",
              run_ordinal: 1,
              expected_seq: 2,
              observed_seq: 1,
            },
          ],
          segments: [unexplainedMissingTail],
        }),
      ).status,
    ).toBe("partial");

    expect(() =>
      parseProductTranscriptResponse(
        transcriptResponse({
          status: "partial",
          partial_reasons: [
            { code: "runtime_run_missing", run_ordinal: 1 },
          ],
          segments: [unexplainedMissingTail],
        }),
      ),
    ).toThrow(ProductApiSchemaError);
  });

  it("rejects transcript status that contradicts partial reasons", () => {
    expect(() =>
      parseProductTranscriptResponse(
        transcriptResponse({
          partial_reasons: [
            { code: "runtime_state_unavailable", run_ordinal: 1 },
          ],
        }),
      ),
    ).toThrow(ProductApiSchemaError);
    expect(() =>
      parseProductTranscriptResponse(
        transcriptResponse({ status: "partial", partial_reasons: [] }),
      ),
    ).toThrow(ProductApiSchemaError);
  });

  it("parses the transcript cursor fields and enforces their invariant", () => {
    const page = parseProductTranscriptResponse(
      transcriptResponse({ has_more: true, next_before_ordinal: 237 }),
    );
    expect(page.has_more).toBe(true);
    expect(page.next_before_ordinal).toBe(237);

    const oldest = parseProductTranscriptResponse(
      transcriptResponse({ has_more: false }),
    );
    expect(oldest.has_more).toBe(false);
    expect(oldest.next_before_ordinal).toBeUndefined();

    // A legacy response carries neither field.
    const legacy = parseProductTranscriptResponse(transcriptResponse());
    expect(legacy.has_more).toBeUndefined();
    expect(legacy.next_before_ordinal).toBeUndefined();

    // `has_more` must equal whether the cursor is present.
    expect(() =>
      parseProductTranscriptResponse(
        transcriptResponse({ has_more: true }),
      ),
    ).toThrow(ProductApiSchemaError);
    expect(() =>
      parseProductTranscriptResponse(
        transcriptResponse({ has_more: false, next_before_ordinal: 5 }),
      ),
    ).toThrow(ProductApiSchemaError);
    expect(() =>
      parseProductTranscriptResponse(
        transcriptResponse({ next_before_ordinal: 5 }),
      ),
    ).toThrow(ProductApiSchemaError);
  });

  it("rejects transcript cursor values that are not positive integers", () => {
    for (const next_before_ordinal of [0, -1, 1.5, "5", true]) {
      expect(() =>
        parseProductTranscriptResponse(
          transcriptResponse({ has_more: true, next_before_ordinal }),
        ),
      ).toThrow(ProductApiSchemaError);
    }
    expect(() =>
      parseProductTranscriptResponse(
        transcriptResponse({ has_more: "yes" }),
      ),
    ).toThrow(ProductApiSchemaError);
  });

  it("relaxes only the leading gap of a cursor page", () => {
    // A cursor page starts where the cursor landed: the runs below its first
    // segment are fetched by an older page, not explained by a partial reason.
    expect(
      parseProductTranscriptResponse(
        transcriptResponse({
          has_more: true,
          next_before_ordinal: 5,
          segments: [transcriptSegment(5), transcriptSegment(6)],
        }),
      ).segments.map((segment) => segment.binding.ordinal),
    ).toEqual([5, 6]);

    // The relaxation keys on the response being a cursor page (`has_more`
    // present), not on its value: the last page is still a page, and the runs
    // below its first segment were simply never returned.
    expect(
      parseProductTranscriptResponse(
        transcriptResponse({
          has_more: false,
          segments: [transcriptSegment(5)],
        }),
      ).has_more,
    ).toBe(false);
    // A gap *inside* the page stays a partial reason even on a cursor page.
    expect(() =>
      parseProductTranscriptResponse(
        transcriptResponse({
          has_more: true,
          next_before_ordinal: 5,
          segments: [transcriptSegment(5), transcriptSegment(7)],
        }),
      ),
    ).toThrow(ProductApiSchemaError);

    // A legacy response keeps the strict leading-gap rule.
    expect(() =>
      parseProductTranscriptResponse(
        transcriptResponse({ segments: [transcriptSegment(5)] }),
      ),
    ).toThrow(ProductApiSchemaError);
  });

  it("accepts a delivered run only when a withheld-row reason covers the hole", () => {
    // The server withholds the rows whose kind this client's declared
    // `event_contract` predates, so the delivered sequence can step over a
    // durable row. Nothing else may create a hole.
    const holed = {
      ...transcriptSegment(),
      observed_through_seq: 3,
      last_event_seq: 3,
      events: [
        {
          seq: 1,
          event: {
            type: "run_started",
            run_id: "01J00000000000000000000006",
            job_id: "01J00000000000000000000005",
            user_message: "hello",
          },
        },
        { seq: 3, event: { type: "llm_chunk", delta: "world" } },
      ],
    };
    const withheld = {
      code: "unknown_event_type",
      run_ordinal: 1,
      run_id: "01J00000000000000000000006",
      expected_seq: 2,
      observed_seq: 2,
    };
    const parsed = parseProductTranscriptResponse(
      transcriptResponse({
        status: "partial",
        partial_reasons: [withheld],
        segments: [holed],
      }),
    );
    expect(parsed.segments[0]!.events.map((event) => event.seq)).toEqual([1, 3]);

    // The same hole without a typed reason is still a schema error, and a reason
    // that names another run, does not cover the step, or is a different code is
    // not a licence either.
    expect(() =>
      parseProductTranscriptResponse(transcriptResponse({ segments: [holed] })),
    ).toThrow(ProductApiSchemaError);
    for (const reason of [
      { ...withheld, run_ordinal: 2 },
      { ...withheld, expected_seq: 3 },
      { ...withheld, observed_seq: 1 },
      { ...withheld, code: "corrupt_event" },
    ]) {
      expect(() =>
        parseProductTranscriptResponse(
          transcriptResponse({
            status: "partial",
            partial_reasons: [reason],
            segments: [holed],
          }),
        ),
      ).toThrow(ProductApiSchemaError);
    }
  });

  it("accepts a withheld tail only with its own typed reason", () => {
    // A withheld final row leaves `observed_through_seq` below `last_event_seq`,
    // which stays a gap until a typed reason for that run explains it.
    const withheldTail = {
      ...transcriptSegment(),
      observed_through_seq: 1,
      last_event_seq: 2,
    };
    expect(
      parseProductTranscriptResponse(
        transcriptResponse({
          status: "partial",
          partial_reasons: [
            { code: "unknown_event_type", run_ordinal: 1, expected_seq: 2, observed_seq: 2 },
          ],
          segments: [withheldTail],
        }),
      ).partial_reasons,
    ).toHaveLength(1);
    expect(() =>
      parseProductTranscriptResponse(
        transcriptResponse({ segments: [withheldTail] }),
      ),
    ).toThrow(ProductApiSchemaError);

    // The withheld reason claims exact durable positions, so it explains a tail
    // only when its range reaches `last_event_seq`. A withheld row at 2 cannot
    // account for a gap that runs to 100, and a client that accepted it could
    // never notice an under-reporting server.
    const underReported = { ...withheldTail, last_event_seq: 100 };
    expect(() =>
      parseProductTranscriptResponse(
        transcriptResponse({
          status: "partial",
          partial_reasons: [
            { code: "unknown_event_type", run_ordinal: 1, expected_seq: 2, observed_seq: 2 },
          ],
          segments: [underReported],
        }),
      ),
    ).toThrow(ProductApiSchemaError);
    // The same tail is still explained by the residue the server reports beside
    // the withheld range, by a positionless code, and by a withheld reason whose
    // range does reach the end.
    for (const reasons of [
      [
        { code: "unknown_event_type", run_ordinal: 1, expected_seq: 2, observed_seq: 2 },
        { code: "missing_event_range", run_ordinal: 1, expected_seq: 3, observed_seq: 100 },
      ],
      [{ code: "corrupt_event", run_ordinal: 1, expected_seq: 2, observed_seq: 2 }],
      [{ code: "unknown_event_type", run_ordinal: 1, expected_seq: 2, observed_seq: 100 }],
    ]) {
      expect(
        parseProductTranscriptResponse(
          transcriptResponse({
            status: "partial",
            partial_reasons: reasons,
            segments: [underReported],
          }),
        ).partial_reasons,
      ).toHaveLength(reasons.length);
    }
    // A reason that names another runtime run is not this run's explanation.
    expect(() =>
      parseProductTranscriptResponse(
        transcriptResponse({
          status: "partial",
          partial_reasons: [
            {
              code: "unknown_event_type",
              run_ordinal: 1,
              run_id: "01J00000000000000000000099",
              expected_seq: 2,
              observed_seq: 100,
            },
          ],
          segments: [underReported],
        }),
      ),
    ).toThrow(ProductApiSchemaError);
    expect(() =>
      parseProductTranscriptResponse(
        transcriptResponse({
          status: "partial",
          partial_reasons: [
            {
              code: "corrupt_event",
              run_ordinal: 1,
              run_id: "01J00000000000000000000099",
              expected_seq: 2,
              observed_seq: 2,
            },
          ],
          segments: [underReported],
        }),
      ),
    ).toThrow(ProductApiSchemaError);
  });

  it("builds the transcript cursor query only from valid options", async () => {
    const urls: string[] = [];
    const client = createProductApiClient({
      fetch: vi.fn(async (input: URL | RequestInfo) => {
        urls.push(String(input));
        return jsonResponse(
          transcriptResponse({ segments: [], has_more: false }),
        );
      }),
    });

    await client.getTranscript(session.id);
    await client.getTranscript(session.id, { limitRuns: 64 });
    await client.getTranscript(session.id, { beforeOrdinal: 45 });
    await client.getTranscript(session.id, { beforeOrdinal: 45, limitRuns: 1 });

    expect(urls.map((url) => url.replace(/^.*\/transcript/u, ""))).toEqual([
      `?event_contract=${STREAM_EVENT_CONTRACT_VERSION}`,
      `?event_contract=${STREAM_EVENT_CONTRACT_VERSION}&limit_runs=64`,
      `?event_contract=${STREAM_EVENT_CONTRACT_VERSION}&before_ordinal=45`,
      `?event_contract=${STREAM_EVENT_CONTRACT_VERSION}&before_ordinal=45&limit_runs=1`,
    ]);
    // The declared contract is the client's explicit capability, never a guess:
    // every transcript request carries it, including the parameterless restore.
    expect(
      urls.every((url) =>
        url.includes(`event_contract=${STREAM_EVENT_CONTRACT_VERSION}`),
      ),
    ).toBe(true);

    const failing = vi.fn();
    const strict = createProductApiClient({ fetch: failing });
    const invalid = [
      { beforeOrdinal: 0 },
      { beforeOrdinal: -3 },
      { beforeOrdinal: 1.5 },
      { limitRuns: 0 },
      { limitRuns: 65 },
      { limitRuns: 2.5 },
    ];
    for (const options of invalid) {
      await expect(strict.getTranscript(session.id, options)).rejects.toBeInstanceOf(
        RangeError,
      );
    }
    expect(failing).not.toHaveBeenCalled();
  });

  it("rejects provider URL credentials, query secrets, and invalid env names before fetch", async () => {
    const fetchMock = vi.fn();
    const client = createProductApiClient({ fetch: fetchMock });

    await expect(
      client.createProviderProfile({
        label: "Unsafe",
        provider_type: "openai",
        api_base: "https://user:secret@gateway.example.test/v1",
        api_key_env: "OPENAI_API_KEY",
      }),
    ).rejects.toBeInstanceOf(ProductApiSchemaError);
    await expect(
      client.createProviderProfile({
        label: "Unsafe",
        provider_type: "openai",
        api_base: "https://gateway.example.test/v1?token=secret",
        api_key_env: "OPENAI_API_KEY",
      }),
    ).rejects.toBeInstanceOf(ProductApiSchemaError);
    await expect(
      client.createProviderProfile({
        label: "Unsafe",
        provider_type: "openai",
        api_base: "https://gateway.example.test/v1",
        api_key_env: "not-valid",
      }),
    ).rejects.toBeInstanceOf(ProductApiSchemaError);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("rejects an API key environment reference for the fake provider", () => {
    expect(() =>
      parseProductProviderProfileRequest({
        label: "Fake",
        provider_type: "fake",
        api_base: "",
        api_key_env: "FAKE_API_KEY",
      }),
    ).toThrow(ProductApiSchemaError);
  });

  it("preserves tool success execution metadata", () => {
    const event = parseStreamEvent({
      type: "tool_call_completed",
      call_id: "call-1",
      result: {
        call_id: "call-1",
        output: "updated",
        metadata: {
          status: "partial_success",
          error_code: "partial_write",
          security_event_type: "workspace_mutation",
          risk_level: "high",
          read_only: false,
          affected_paths: ["src/main.rs"],
          workspace_changed: true,
          diff_summary: ["updated src/main.rs"],
        },
      },
    });

    expect(event).toEqual({
      type: "tool_call_completed",
      call_id: "call-1",
      result: {
        call_id: "call-1",
        output: "updated",
        metadata: {
          status: "partial_success",
          error_code: "partial_write",
          security_event_type: "workspace_mutation",
          risk_level: "high",
          read_only: false,
          affected_paths: ["src/main.rs"],
          workspace_changed: true,
          diff_summary: ["updated src/main.rs"],
        },
      },
    });
  });

  it("preserves tool failure execution metadata", () => {
    const event = parseStreamEvent({
      type: "tool_call_failed",
      call_id: "call-2",
      error: {
        code: "permission_denied",
        reason: "approval rejected",
      },
      metadata: {
        status: "rejected",
        error_code: "permission_denied",
        security_event_type: "approval_rejected",
        risk_level: "high",
        read_only: false,
        affected_paths: [],
        workspace_changed: false,
        diff_summary: [],
      },
    });

    expect(event).toMatchObject({
      type: "tool_call_failed",
      metadata: {
        status: "rejected",
        error_code: "permission_denied",
        security_event_type: "approval_rejected",
        risk_level: "high",
        read_only: false,
        affected_paths: [],
        workspace_changed: false,
        diff_summary: [],
      },
    });
  });

  it("defaults omitted tool execution metadata and rejects explicit null", () => {
    expect(
      parseStreamEvent({
        type: "tool_call_completed",
        call_id: "call-default",
        result: {
          call_id: "call-default",
          output: "unchanged",
        },
      }),
    ).toMatchObject({
      result: {
        metadata: {
          status: "ok",
          risk_level: "low",
          read_only: false,
          affected_paths: [],
          workspace_changed: false,
          diff_summary: [],
        },
      },
    });
    expect(() =>
      parseStreamEvent({
        type: "tool_call_failed",
        call_id: "call-null",
        error: { code: "execution_failed" },
        metadata: null,
      }),
    ).toThrow(ProductApiSchemaError);
  });

  it("accepts prompt metadata without a prompt cache key", () => {
    const event = parseStreamEvent({
      type: "prompt_built",
      metadata: {
        prompt_hash: "sha256:prompt",
        stable_prefix_hash: "sha256:prefix",
        workspace_fingerprint: "sha256:workspace",
        tool_signature: "sha256:tools",
        token_estimate: 42,
        included_history_messages: 3,
        dropped_history_messages: 1,
      },
    });

    expect(event).toEqual({
      type: "prompt_built",
      metadata: {
        prompt_hash: "sha256:prompt",
        stable_prefix_hash: "sha256:prefix",
        workspace_fingerprint: "sha256:workspace",
        tool_signature: "sha256:tools",
        token_estimate: 42,
        included_history_messages: 3,
        dropped_history_messages: 1,
      },
    });
  });

  it("strictly parses Agent lifecycle events", () => {
    const profileHash = `sha256:${"c".repeat(64)}`;
    const procedure = {
      id: "inspect.disk",
      version: "1.0.0",
      trust: "workspace_trusted",
      source_path: "procedures/inspect.disk.md",
      content_hash: `sha256:${"e".repeat(64)}`,
    } as const;

    expect(
      parseStreamEvent({
        type: "agent_profile_activated",
        identity: {
          selector: { source: "workspace", agent_id: "ops" },
          agent_id: "ops",
          display_name: "Operations",
          definition_version: "1.0.0",
          manifest_hash: `sha256:${"a".repeat(64)}`,
          package_hash: `sha256:${"b".repeat(64)}`,
          profile_hash: profileHash,
          instruction_bundle_hash: `sha256:${"d".repeat(64)}`,
          procedures: [procedure],
        },
        resumed_from_snapshot: false,
        diagnostics: [
          {
            code: "procedure_excluded",
            subject: "procedures/retired.md",
            message: "retired procedure was excluded",
          },
        ],
      }),
    ).toMatchObject({
      type: "agent_profile_activated",
      identity: { profile_hash: profileHash, procedures: [procedure] },
      resumed_from_snapshot: false,
    });

    expect(
      parseStreamEvent({
        type: "workspace_instructions_resolved",
        bundle_hash: `sha256:${"d".repeat(64)}`,
        layer_count: 2,
        rejected_count: 1,
        truncated: false,
      }),
    ).toMatchObject({ type: "workspace_instructions_resolved", layer_count: 2 });
    expect(
      parseStreamEvent({
        type: "instruction_overlay_applied",
        target_path: "apps/web/page.tsx",
        scope: "apps/web",
        source_path: "apps/web/AGENTS.md",
        content_hash: `sha256:${"f".repeat(64)}`,
        boundary: "tool_call",
        call_id: "01JOVERLAY",
      }),
    ).toMatchObject({
      type: "instruction_overlay_applied",
      scope: "apps/web",
      target_path: "apps/web/page.tsx",
    });
    expect(
      parseStreamEvent({
        type: "procedures_selected",
        profile_hash: profileHash,
        selected: [procedure],
        considered_count: 3,
        excluded_count: 2,
      }),
    ).toMatchObject({ type: "procedures_selected", selected: [procedure] });
    expect(
      parseStreamEvent({
        type: "procedure_hydrated",
        reference: procedure,
        truncated: true,
        dropped_bytes: 16,
      }),
    ).toMatchObject({ type: "procedure_hydrated", reference: procedure });
  });

  it("rejects malformed Agent lifecycle events", () => {
    expect(() =>
      parseStreamEvent({
        type: "instruction_overlay_applied",
        target_path: "apps/web/page.tsx",
        scope: "apps/web\nforged",
        source_path: "apps/web/AGENTS.md",
        content_hash: "sha256:f",
        boundary: "tool_call",
      }),
    ).toThrow(ProductApiSchemaError);
    expect(() =>
      parseStreamEvent({
        type: "agent_profile_activated",
        identity: {
          selector: { source: "remote", agent_id: "ops" },
          agent_id: "ops",
          display_name: "Operations",
          definition_version: "1.0.0",
          manifest_hash: "sha256:a",
          package_hash: "sha256:b",
          profile_hash: "sha256:c",
        },
        resumed_from_snapshot: false,
      }),
    ).toThrow(ProductApiSchemaError);
    expect(() =>
      parseStreamEvent({
        type: "procedure_hydrated",
        reference: {
          id: "inspect.disk",
          version: "1.0.0",
          trust: "workspace_trusted",
          source_path: "procedures/inspect.disk.md",
          content_hash: "sha256:e",
          permission: "allow",
        },
        truncated: false,
        dropped_bytes: -1,
      }),
    ).toThrow(ProductApiSchemaError);
  });

  it("strictly parses bounded MCP lifecycle events", () => {
    expect(
      parseStreamEvent({
        type: "mcp_server_degraded",
        server_config_id: "monitoring",
        required: false,
        failure_code: "mcp_catalog_refresh_failed",
      }),
    ).toEqual({
      type: "mcp_server_degraded",
      server_config_id: "monitoring",
      required: false,
      failure_code: "mcp_catalog_refresh_failed",
    });
    expect(
      parseStreamEvent({
        type: "mcp_capabilities_refreshed",
        server_config_id: "monitoring",
        snapshot_id: "sha256:catalog-v2",
        added: ["mcp__monitoring__new"],
        removed: ["mcp__monitoring__retired"],
        changed: ["mcp__monitoring__query"],
      }),
    ).toMatchObject({
      type: "mcp_capabilities_refreshed",
      added: ["mcp__monitoring__new"],
      removed: ["mcp__monitoring__retired"],
      changed: ["mcp__monitoring__query"],
    });
  });

  it("rejects control characters and oversized MCP lifecycle diffs", () => {
    expect(() =>
      parseStreamEvent({
        type: "mcp_server_degraded",
        server_config_id: "monitoring",
        required: false,
        failure_code: "mcp_failed\nforged trace",
      }),
    ).toThrow(ProductApiSchemaError);
    expect(() =>
      parseStreamEvent({
        type: "mcp_capabilities_refreshed",
        server_config_id: "monitoring",
        snapshot_id: "sha256:catalog-v2",
        added: Array.from({ length: 129 }, (_, index) => `tool_${index}`),
        removed: [],
        changed: [],
      }),
    ).toThrow(ProductApiSchemaError);
  });

  it("keeps the legacy promote body byte-identical and adds a delivery only on request", async () => {
    const queued = {
      id: control.id,
      product_session_id: session.id,
      content: "wait for the boundary",
      requested_delivery: "successor",
      status: "queued",
      seq: 7,
      created_at: "2026-07-26T00:00:00.000Z",
    };
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, _init?: RequestInit) => jsonResponse(queued),
    );
    const client = createProductApiClient({ fetch: fetchMock });

    await client.promoteMessage(session.id, control.id);
    await client.promoteMessage(session.id, control.id, { delivery: "successor" });
    await client.promoteMessage(session.id, control.id, {});

    expect(String(fetchMock.mock.calls[0]?.[0])).toBe(
      `/api/product/sessions/${session.id}/messages/${control.id}/promote`,
    );
    // A parameterless promotion is exactly the historical request.
    expect(fetchMock.mock.calls[0]?.[1]?.body).toBe("{}");
    expect(fetchMock.mock.calls[2]?.[1]?.body).toBe("{}");
    expect(fetchMock.mock.calls[1]?.[1]?.body).toBe('{"delivery":"successor"}');
  });

  it("reorders the successor queue in one request and refuses a list the server would reject", async () => {
    const queued = (id: string, seq: number) => ({
      id,
      product_session_id: session.id,
      content: `queued ${seq}`,
      requested_delivery: "successor",
      status: "queued",
      seq,
      queue_order: seq,
      created_at: "2026-07-26T00:00:00.000Z",
    });
    const first = "01J00000000000000000000011";
    const second = "01J00000000000000000000012";
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, _init?: RequestInit) =>
        jsonResponse({ messages: [queued(second, 2), queued(first, 1)] }),
    );
    const client = createProductApiClient({ fetch: fetchMock });

    const response = await client.reorderMessages(session.id, {
      ordered_ids: [second, first],
    });

    expect(String(fetchMock.mock.calls[0]?.[0])).toBe(
      `/api/product/sessions/${session.id}/messages/reorder`,
    );
    expect(fetchMock.mock.calls[0]?.[1]?.method).toBe("POST");
    expect(fetchMock.mock.calls[0]?.[1]?.body).toBe(
      `{"ordered_ids":["${second}","${first}"]}`,
    );
    expect(response.messages.map((message) => message.id)).toEqual([second, first]);
    expect(response.messages[0]?.queue_order).toBe(2);

    // A duplicated id is a list the endpoint refuses, so it never leaves here,
    // and neither does a list longer than the server's bound.
    await expect(
      client.reorderMessages(session.id, { ordered_ids: [first, first] }),
    ).rejects.toBeInstanceOf(ProductApiSchemaError);
    await expect(
      client.reorderMessages(session.id, {
        ordered_ids: Array.from(
          { length: MAX_PENDING_PRODUCT_MESSAGES + 1 },
          (_, index) => `01J0000000000000000000${String(index).padStart(4, "0")}`,
        ),
      }),
    ).rejects.toBeInstanceOf(ProductApiSchemaError);
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("compacts the idle session and refuses a response bound to another session", async () => {
    const compaction = {
      product_session_id: session.id,
      runtime_run_id: "01J00000000000000000000006",
      triggered: true,
      mode: "model_generated",
      degraded: false,
      consecutive_failures: 0,
      circuit_open: false,
      source_message_count: 12,
      summary_truncated: false,
      token_estimate: 900,
      summary: "bounded excerpt",
    };
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, _init?: RequestInit) => jsonResponse(compaction),
    );
    const client = createProductApiClient({ fetch: fetchMock });

    const response = await client.compactSession(session.id);

    expect(String(fetchMock.mock.calls[0]?.[0])).toBe(
      `/api/product/sessions/${session.id}/compact`,
    );
    expect(fetchMock.mock.calls[0]?.[1]?.method).toBe("POST");
    expect(fetchMock.mock.calls[0]?.[1]?.body).toBe("{}");
    expect(response.summary).toBe("bounded excerpt");
    expect(response.failure_code).toBeUndefined();

    const wrong = createProductApiClient({
      fetch: vi.fn(async () =>
        jsonResponse({ ...compaction, product_session_id: forkedSession.id })),
    });
    await expect(wrong.compactSession(session.id)).rejects.toBeInstanceOf(
      ProductApiSchemaError,
    );
  });

  it("never invents a missing compaction fact", async () => {
    const minimal = {
      product_session_id: session.id,
      triggered: false,
      mode: "disabled",
      degraded: false,
      consecutive_failures: 2,
      circuit_open: true,
      source_message_count: 0,
      summary_truncated: false,
      token_estimate: 0,
    };
    const client = createProductApiClient({
      fetch: vi.fn(async () => jsonResponse(minimal)),
    });

    const response = await client.compactSession(session.id);

    // No run means "there is nothing to compact" and the response says so by
    // omission; nothing is filled in on its behalf.
    expect(response.runtime_run_id).toBeUndefined();
    expect(response.summary).toBeUndefined();
    expect(response.failure_code).toBeUndefined();
    expect(response.next_attempt_after).toBeUndefined();
    expect(response.model).toBeUndefined();
    expect(response.prompt_version).toBeUndefined();
    expect(isKnownProductCompactionMode(response.mode)).toBe(true);
  });

  it("encodes both search routes and refuses a term or cursor the server would reject", async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL) =>
      String(input).includes("/sessions/")
        ? jsonResponse({ hits: [], next_cursor: "c1" })
        : jsonResponse({ scope: `session:${session.id}`, hits: [] }));
    const client = createProductApiClient({ fetch: fetchMock });

    const sessionPage = await client.searchSessionMessages(session.id, {
      q: "needle",
      limit: 32,
    });
    const corpusPage = await client.searchProduct({
      q: "needle",
      scope: `session:${session.id}`,
    });

    expect(String(fetchMock.mock.calls[0]?.[0])).toBe(
      `/api/product/sessions/${session.id}/search?q=needle&limit=32`,
    );
    expect(String(fetchMock.mock.calls[1]?.[0])).toBe(
      `/api/product/search?q=needle&scope=session%3A${session.id}`,
    );
    expect(sessionPage.next_cursor).toBe("c1");
    expect(corpusPage.scope).toBe(`session:${session.id}`);

    // A refused term is refused before it leaves the browser, because the server
    // answers a blank or oversized term with 400 and the UI must not present a
    // refusal as "no results".
    await expect(
      client.searchSessionMessages(session.id, { q: "   " }),
    ).rejects.toBeInstanceOf(RangeError);
    await expect(
      client.searchSessionMessages(session.id, { q: "a".repeat(129) }),
    ).rejects.toBeInstanceOf(RangeError);
    await expect(
      client.searchSessionMessages(session.id, { q: "a\u0000b" }),
    ).rejects.toBeInstanceOf(RangeError);
    await expect(
      client.searchSessionMessages(session.id, { q: "needle", limit: 0 }),
    ).rejects.toBeInstanceOf(RangeError);
    await expect(
      client.searchSessionMessages(session.id, { q: "needle", limit: 101 }),
    ).rejects.toBeInstanceOf(RangeError);
    await expect(
      client.searchSessionMessages(session.id, { q: "needle", cursor: "" }),
    ).rejects.toBeInstanceOf(RangeError);
    await expect(
      client.searchProduct({ q: "needle", scope: "conversation:1" }),
    ).rejects.toBeInstanceOf(RangeError);
    await expect(
      client.searchProduct({ q: "needle", scope: "trace:" }),
    ).rejects.toBeInstanceOf(RangeError);
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it("parses a search page without inventing a timestamp or a message binding", () => {
    const response = parseProductSearchResponse({
      scope: "trace:01J00000000000000000000002",
      hits: [
        {
          session_id: session.id,
          source: "trace",
          seq: 12,
          run_id: "01J00000000000000000000006",
          run_ordinal: 3,
          snippet: '{"type":"run_started"}',
        },
      ],
    });

    // A legacy trace line has no `created_at`; supplying one would be a
    // fabricated fact, so the key stays absent for the view to render as such.
    expect(response.hits[0]).not.toHaveProperty("created_at");
    expect(response.next_cursor).toBeUndefined();
    expect(response.hits[0]?.source).toBe("trace");

    expect(() =>
      parseProductSearchResponse({
        scope: "trace:01J00000000000000000000002",
        hits: [
          {
            session_id: session.id,
            source: "trace",
            seq: 12,
            run_ordinal: -1,
            snippet: "x",
          },
        ],
      }),
    ).toThrow(ProductApiSchemaError);
    expect(() =>
      parseProductSearchResponse({
        scope: "trace:01J00000000000000000000002",
        hits: [
          { session_id: session.id, source: "document", seq: 1, snippet: "x" },
        ],
      }),
    ).toThrow(ProductApiSchemaError);
  });

  it("keeps an unknown compaction mode and a typed failure code verbatim", () => {
    const compaction = parseProductSessionCompaction({
      product_session_id: session.id,
      runtime_run_id: "01J00000000000000000000006",
      triggered: true,
      mode: "hybrid_preview",
      degraded: true,
      consecutive_failures: 3,
      circuit_open: true,
      next_attempt_after: "2026-07-26T00:05:00.000Z",
      source_message_count: 4,
      summary: "excerpt",
      summary_truncated: true,
      token_estimate: 120,
      failure_code: "summary_timeout",
    });

    // A mode a newer server adds must render as itself, not as a guess.
    expect(isKnownProductCompactionMode(compaction.mode)).toBe(false);
    expect(compaction.mode).toBe("hybrid_preview");
    expect(compaction.failure_code).toBe("summary_timeout");
    expect(compaction.summary_truncated).toBe(true);
    expect(() =>
      parseProductSessionCompaction({
        product_session_id: session.id,
        triggered: true,
        degraded: false,
        consecutive_failures: 0,
        circuit_open: false,
        source_message_count: 0,
        summary_truncated: false,
        token_estimate: 0,
      }),
    ).toThrow(ProductApiSchemaError);
    expect(() =>
      parseProductSessionCompaction({
        product_session_id: session.id,
        triggered: true,
        mode: "model_generated\nforged",
        degraded: false,
        consecutive_failures: 0,
        circuit_open: false,
        source_message_count: 0,
        summary_truncated: false,
        token_estimate: 0,
      }),
    ).toThrow(ProductApiSchemaError);
  });
});

describe("attachment transport", () => {
  const attachment = {
    attachment_id: "01J00000000000000000000031",
    product_session_id: session.id,
    content_type: "image/png",
    size: 2048,
    sha256: "a".repeat(64),
    name: "shot.png",
    status: "staged",
    warnings: [],
  };

  it("uploads the file as the raw body with a claim derived from its name", async () => {
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, _init?: RequestInit) => jsonResponse(attachment, 201),
    );
    const client = createProductApiClient({ fetch: fetchMock });
    const file = new File([new Uint8Array([1, 2, 3])], "shot.png", {
      type: "application/octet-stream",
    });

    await expect(
      client.uploadAttachment(session.id, file, { name: "shot.png" }),
    ).resolves.toEqual(attachment);

    const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(String(url)).toBe(
      `/api/product/sessions/${session.id}/attachments?name=shot.png`,
    );
    expect(init.method).toBe("POST");
    // The body is the bytes themselves; a multipart envelope would be a second
    // wire format for no gain.
    expect(init.body).toBe(file);
    expect(new Headers(init.headers).get("content-type")).toBe("image/png");
  });

  it("falls back to the declared type for a name the allow-list cannot type", async () => {
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, _init?: RequestInit) => jsonResponse(attachment, 201),
    );
    const client = createProductApiClient({ fetch: fetchMock });
    const file = new File([new Uint8Array([1])], "blob", {
      type: "application/octet-stream",
    });

    await client.uploadAttachment(session.id, file);

    const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(String(url)).toBe(`/api/product/sessions/${session.id}/attachments`);
    expect(new Headers(init.headers).get("content-type")).toBe(
      "application/octet-stream",
    );
  });

  it("surfaces a typed upload refusal instead of a generic failure", async () => {
    const client = createProductApiClient({
      fetch: vi.fn(async () =>
        jsonResponse({ code: "product_attachment_too_large", error: "too large" }, 413),
      ),
    });
    await expect(
      client.uploadAttachment(session.id, new File([new Uint8Array([1])], "a.pdf")),
    ).rejects.toMatchObject({ status: 413, code: "product_attachment_too_large" });
  });

  it("rejects a malformed upload body rather than trusting it", async () => {
    const client = createProductApiClient({
      fetch: vi.fn(async () => jsonResponse({ ...attachment, warnings: undefined }, 201)),
    });
    await expect(
      client.uploadAttachment(session.id, new File([new Uint8Array([1])], "a.pdf")),
    ).rejects.toBeInstanceOf(ProductApiSchemaError);
  });

  it("fetches attachment bytes through the authenticated transport", async () => {
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, _init?: RequestInit) =>
        new Response(new Uint8Array([1, 2, 3]), {
          status: 200,
          headers: { "content-type": "image/png" },
        }),
    );
    const client = createProductApiClient({ fetch: fetchMock });

    const blob = await client.fetchAttachment(session.id, attachment.attachment_id);

    expect(blob.size).toBe(3);
    expect(String(fetchMock.mock.calls[0]?.[0])).toBe(
      `/api/product/sessions/${session.id}/attachments/${attachment.attachment_id}`,
    );
  });

  it("probes a reference with HEAD and maps the status onto a typed code", async () => {
    const ok = vi.fn(
      async (_input: RequestInfo | URL, _init?: RequestInit) =>
        new Response(null, { status: 200 }),
    );
    await expect(
      createProductApiClient({ fetch: ok }).probeAttachment(
        session.id,
        attachment.attachment_id,
      ),
    ).resolves.toBeUndefined();
    expect(String(ok.mock.calls[0]?.[0])).toBe(
      `/api/product/sessions/${session.id}/attachments/${attachment.attachment_id}`,
    );
    expect((ok.mock.calls[0]?.[1] as RequestInit).method).toBe("HEAD");

    // A probe has no body, so the code the GET body would carry is restored
    // from the status.
    const cases: [number, string][] = [
      [404, "product_attachment_not_found"],
      [409, "product_attachment_conflict"],
      [410, "product_attachment_unavailable"],
      [429, "product_attachment_busy"],
      [500, "http_error"],
    ];
    for (const [status, code] of cases) {
      const failing = vi.fn(
        async (_input: RequestInfo | URL, _init?: RequestInit) =>
          new Response(null, { status }),
      );
      await expect(
        createProductApiClient({ fetch: failing }).probeAttachment(
          session.id,
          attachment.attachment_id,
        ),
      ).rejects.toMatchObject({ status, code });
    }
  });

  it("authenticates the Desktop transport for attachments", async () => {
    vi.stubGlobal("window", {
      __ROVE_API_URL__: "http://127.0.0.1:49152",
      __ROVE_TOKEN__: "desktop-secret",
    });
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, _init?: RequestInit) =>
        new Response(new Uint8Array([1]), {
          status: 200,
          headers: { "content-type": "image/png" },
        }),
    );

    await createProductApiClient({ fetch: fetchMock }).fetchAttachment(
      session.id,
      attachment.attachment_id,
    );

    expect(String(fetchMock.mock.calls[0]?.[0])).toBe(
      `http://127.0.0.1:49152/product/sessions/${session.id}/attachments/${attachment.attachment_id}`,
    );
    expect(
      new Headers((fetchMock.mock.calls[0]?.[1] as RequestInit).headers).get(
        "authorization",
      ),
    ).toBe("Bearer desktop-secret");
  });

  it("sends attachment references with a message", async () => {
    const accepted = {
      id: control.id,
      product_session_id: session.id,
      content: "look",
      requested_delivery: "successor",
      status: "queued",
      seq: 1,
      created_at: "2026-09-26T12:00:00.000Z",
      attachments: [
        {
          attachment_id: attachment.attachment_id,
          content_type: "image/png",
          size: 2048,
          sha256: "a".repeat(64),
          availability: "available",
        },
      ],
    };
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, _init?: RequestInit) => jsonResponse(accepted),
    );
    const client = createProductApiClient({ fetch: fetchMock });

    const message = await client.sendMessage(session.id, {
      content: "look",
      idempotency_key: "key-1",
      attachments: [{ attachment_id: attachment.attachment_id, name: "shot.png" }],
    });

    const [, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(JSON.parse(String(init.body))).toEqual({
      content: "look",
      idempotency_key: "key-1",
      attachments: [{ attachment_id: attachment.attachment_id, name: "shot.png" }],
    });
    expect(message.attachments?.map((ref) => ref.attachment_id)).toEqual([
      attachment.attachment_id,
    ]);
  });

  it("onboards a provider credential through the loopback endpoint", async () => {
    const calls: Array<{ url: string; method: string; body?: string }> = [];
    const receipt = {
      profile_id: providerProfile.id,
      label: "Gateway",
      provider_type: "openai",
      api_base: "https://gateway.example.test/v1",
      model: "test/model",
      catalog_revision: "sha256:provider-catalog-2",
      credential_source: "keyring",
      probe: {
        inventory_count: 3,
        streaming_supported: true,
        native_tool_calls_supported: true,
        usage_supported: false,
      },
      selected: true,
    };
    const fetchMock: typeof globalThis.fetch = vi.fn(
      async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({
          url: String(input),
          method: init?.method ?? "GET",
          body: typeof init?.body === "string" ? init.body : undefined,
        });
        return jsonResponse(receipt, 201);
      },
    );
    const client = createProductApiClient({ fetch: fetchMock });

    const result = await client.onboardProvider({
      label: "Gateway",
      provider_type: "openai",
      api_base: "https://gateway.example.test/v1",
      model: "test/model",
      make_default: true,
      credential: "sk-test-only",
    });

    expect(calls[0]?.url).toBe("/api/product/provider-onboarding");
    expect(calls[0]?.method).toBe("POST");
    expect(JSON.parse(calls[0]?.body ?? "")).toEqual({
      label: "Gateway",
      provider_type: "openai",
      api_base: "https://gateway.example.test/v1",
      model: "test/model",
      make_default: true,
      credential: "sk-test-only",
    });
    // The parsed receipt carries no secret material.
    expect(result).toEqual(receipt);
    expect(JSON.stringify(result)).not.toContain("sk-test-only");
  });

  it("rejects an onboarding request with a blank credential before fetch", async () => {
    const fetchMock = vi.fn(async () => jsonResponse({}));
    const client = createProductApiClient({ fetch: fetchMock });

    await expect(
      client.onboardProvider({
        label: "Gateway",
        provider_type: "openai",
        api_base: "https://gateway.example.test/v1",
        model: "test/model",
        credential: "   ",
      }),
    ).rejects.toBeInstanceOf(ProductApiSchemaError);
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
