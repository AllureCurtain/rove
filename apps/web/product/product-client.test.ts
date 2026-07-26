import { describe, expect, it, vi } from "vitest";

import {
  ProductApiSchemaError,
  parseProductTranscriptResponse,
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
  theme: "dark",
  active_workspace_id: workspace.id,
  active_session_id: session.id,
  provider_selection: {
    profile_id: providerProfile.id,
    model: "test/model",
    approval: "ask",
    max_steps: 8,
  },
};

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
      theme: "dark",
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
});
