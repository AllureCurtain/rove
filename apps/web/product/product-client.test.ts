import { afterEach, describe, expect, it, vi } from "vitest";

import {
  ProductApiSchemaError,
  PRODUCT_ERROR_CODES,
  parseCreateProductControlRequest,
  parseProductControl,
  parseProductPreferences,
  parseProductProviderProfileRequest,
  parseProductTranscriptResponse,
  parseUpdateProductPreferencesRequest,
  parseStreamEvent,
} from "./product-api-types";
import { createProductApiClient } from "./product-client";

const workspace = {
  id: "01J00000000000000000000001",
  canonical_root: "D:\\Study\\project\\agent\\rove",
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
        if (url.endsWith(`/product/sessions/${session.id}`)) {
          return jsonResponse({ ...session, title: "Renamed" });
        }
        if (url.endsWith(`/product/sessions/${session.id}/transcript`)) {
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
      `PATCH /api/product/sessions/${session.id}`,
      `GET /api/product/sessions/${session.id}/transcript`,
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
});
