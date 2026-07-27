import { describe, expect, it, vi } from "vitest";

import {
  ProductApiSchemaError,
  PRODUCT_ERROR_CODES,
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

const providerProfile = {
  id: "01J00000000000000000000003",
  label: "Gateway",
  provider_type: "openai",
  api_base: "https://gateway.example.test/v1",
  api_key_env: "GATEWAY_API_KEY",
  default_model: "test/model",
  created_at: "2026-07-26T00:00:00.000Z",
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

describe("product API client", () => {
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
        if (url === "/api/product/provider-profiles" && method === "GET") {
          return jsonResponse({ provider_profiles: [providerProfile] });
        }
        if (url === "/api/product/provider-profiles" && method === "POST") {
          return jsonResponse(providerProfile, 201);
        }
        if (url.endsWith(`/product/provider-profiles/${providerProfile.id}`)) {
          return jsonResponse({ ...providerProfile, label: "Updated" });
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
    await client.updateSession(session.id, { title: "Renamed" });
    await client.getTranscript(session.id);
    await client.deleteSession(session.id);
    await client.listProviderProfiles();
    await client.createProviderProfile({
      label: "Gateway",
      provider_type: "openai",
      api_base: "https://gateway.example.test/v1",
      api_key_env: "GATEWAY_API_KEY",
      default_model: "test/model",
    });
    await client.updateProviderProfile(providerProfile.id, {
      label: "Updated",
      provider_type: "openai",
      api_base: "https://gateway.example.test/v1",
      api_key_env: "GATEWAY_API_KEY",
      default_model: "test/model",
    });
    await client.deleteProviderProfile(providerProfile.id);
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
      `PATCH /api/product/sessions/${session.id}`,
      `GET /api/product/sessions/${session.id}/transcript`,
      `DELETE /api/product/sessions/${session.id}`,
      "GET /api/product/provider-profiles",
      "POST /api/product/provider-profiles",
      `PUT /api/product/provider-profiles/${providerProfile.id}`,
      `DELETE /api/product/provider-profiles/${providerProfile.id}`,
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
});
