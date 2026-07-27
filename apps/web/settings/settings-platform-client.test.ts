import { describe, expect, it, vi } from "vitest";

import { ProductApiSchemaError } from "../product/product-api-types";
import { ProductApiError } from "../product/product-client";
import {
  parseProductMemoryTopicContentResponse,
  parseProductMemoryTopicsResponse,
  parseProductRuntimeInfo,
  parseSettingsPreferencesUpdateRequest,
  type SettingsPreferencesUpdateRequest,
} from "./settings-platform-api-types";
import { createSettingsPlatformClient } from "./settings-platform-client";

const topic = {
  slug: "project-conventions",
  title: "Project Conventions",
  memory_type: "project",
  scope: "project",
  confidence: 0.8,
  created_at: "2026-07-27T00:00:00Z",
  updated_at: "2026-07-27T01:00:00Z",
  description: "project memory",
  metadata_truncated: false,
} as const;

const preferences = {
  schema_version: 1,
  revision: 7,
  theme: "dark",
  default_approval_policy: "ask",
  active_workspace_id: "01J00000000000000000000001",
  active_session_id: "01J00000000000000000000002",
  provider_selection: {
    profile_id: "01J00000000000000000000003",
    model: "test/model",
    approval: "ask",
    max_steps: 8,
  },
} as const;

const runtimeInfo = {
  api_version: "0.1.0",
  connection: "connected",
  product_store: "ready",
  resume_health: {
    status: "healthy",
    workspace_count: 1,
    session_count: 1,
    bound_session_count: 1,
    running_session_count: 0,
    needs_attention_session_count: 0,
  },
} as const;

function jsonResponse(value: unknown, status = 200): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json" },
  });
}

describe("settings platform API types", () => {
  it("strictly parses bounded memory and runtime responses", () => {
    expect(
      parseProductMemoryTopicsResponse({ topics: [topic], total: 1 }),
    ).toEqual({ topics: [topic], total: 1 });
    expect(
      parseProductMemoryTopicContentResponse({
        topic,
        content: "Run cargo fmt before committing.\n",
        truncated: false,
      }),
    ).toMatchObject({ topic, truncated: false });
    expect(parseProductRuntimeInfo(runtimeInfo)).toEqual(runtimeInfo);
    expect(
      parseProductMemoryTopicsResponse({
        topics: [{ ...topic, slug: "\u0345-memory" }],
        total: 1,
      }).topics[0]?.slug,
    ).toBe("\u0345-memory");
    expect(
      parseProductRuntimeInfo({
        api_version: "0.1.0",
        connection: "connected",
        product_store: "unavailable",
      }),
    ).toEqual({
      api_version: "0.1.0",
      connection: "connected",
      product_store: "unavailable",
    });
  });

  it("rejects unknown, inconsistent, and oversized platform payloads", () => {
    expect(() =>
      parseProductMemoryTopicsResponse({
        topics: [{ ...topic, secret_path: "C:\\private" }],
        total: 1,
      }),
    ).toThrow(ProductApiSchemaError);
    expect(() =>
      parseProductMemoryTopicsResponse({ topics: [topic], total: 2 }),
    ).toThrow(ProductApiSchemaError);
    expect(() =>
      parseProductMemoryTopicContentResponse({
        topic,
        content: "x".repeat(64 * 1_024 + 1),
        truncated: true,
      }),
    ).toThrow(ProductApiSchemaError);
    expect(() =>
      parseProductRuntimeInfo({
        ...runtimeInfo,
        resume_health: {
          ...runtimeInfo.resume_health,
          status: "healthy",
          needs_attention_session_count: 1,
        },
      }),
    ).toThrow(ProductApiSchemaError);
    expect(() =>
      parseProductRuntimeInfo({
        api_version: "0.1.0",
        connection: "connected",
        product_store: "ready",
      }),
    ).toThrow(ProductApiSchemaError);
  });

  it("requires preference CAS and synchronizes provider approval", () => {
    const parsed = parseSettingsPreferencesUpdateRequest({
      schema_version: 1,
      expected_revision: 7,
      theme: "dark",
      default_approval_policy: "never",
      provider_selection: {
        profile_id: preferences.provider_selection.profile_id,
        model: "test/model",
        approval: "ask",
        max_steps: 8,
      },
    });

    expect(parsed.expected_revision).toBe(7);
    expect(parsed.default_approval_policy).toBe("never");
    expect(parsed.provider_selection?.approval).toBe("never");
    expect(() =>
      parseSettingsPreferencesUpdateRequest({
        schema_version: 1,
        theme: "dark",
        default_approval_policy: "ask",
      }),
    ).toThrow(ProductApiSchemaError);
  });
});

describe("settings platform client", () => {
  it("covers memory, runtime, preferences, and an atomic policy update", async () => {
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
        if (
          url === "/api/product/memory/topics?workspace_id=workspace-1" &&
          method === "GET"
        ) {
          return jsonResponse({ topics: [topic], total: 1 });
        }
        if (
          url ===
            "/api/product/memory/topics/project-conventions?workspace_id=workspace-1" &&
          method === "GET"
        ) {
          return jsonResponse({ topic, content: "Durable fact.\n", truncated: false });
        }
        if (
          url ===
            "/api/product/memory/topics/project-conventions?workspace_id=workspace-1" &&
          method === "DELETE"
        ) {
          return new Response(null, { status: 204 });
        }
        if (url === "/api/product/runtime") {
          return jsonResponse(runtimeInfo);
        }
        if (url === "/api/product/preferences" && method === "GET") {
          return jsonResponse(preferences);
        }
        if (url === "/api/product/preferences" && method === "PUT") {
          const request = JSON.parse(String(init?.body)) as Record<
            string,
            unknown
          >;
          const selection = request.provider_selection as Record<
            string,
            unknown
          >;
          return jsonResponse({
            ...preferences,
            revision: 8,
            default_approval_policy: request.default_approval_policy,
            provider_selection: {
              ...preferences.provider_selection,
              approval: selection.approval,
            },
          });
        }
        return jsonResponse({ code: "not_found", error: "unexpected route" }, 404);
      },
    );
    const client = createSettingsPlatformClient({ fetch: fetchMock });

    await client.listMemoryTopics("workspace-1");
    await client.getMemoryTopic("workspace-1", "project-conventions");
    await client.deleteMemoryTopic("workspace-1", "project-conventions");
    await client.getRuntimeInfo();
    const current = await client.getPreferences();
    const updated = await client.updateDefaultApprovalPolicy(current, "never");

    expect(updated.default_approval_policy).toBe("never");
    expect(updated.provider_selection?.approval).toBe("never");
    expect(calls.map(({ url, method }) => `${method} ${url}`)).toEqual([
      "GET /api/product/memory/topics?workspace_id=workspace-1",
      "GET /api/product/memory/topics/project-conventions?workspace_id=workspace-1",
      "DELETE /api/product/memory/topics/project-conventions?workspace_id=workspace-1",
      "GET /api/product/runtime",
      "GET /api/product/preferences",
      "PUT /api/product/preferences",
    ]);
    const updateBody = JSON.parse(calls.at(-1)?.body ?? "{}") as Record<
      string,
      unknown
    >;
    expect(updateBody.expected_revision).toBe(7);
    expect(updateBody.default_approval_policy).toBe("never");
    expect(updateBody.provider_selection).toMatchObject({ approval: "never" });
  });

  it("rejects missing CAS and unsafe slugs before fetch", async () => {
    const fetchMock = vi.fn();
    const client = createSettingsPlatformClient({ fetch: fetchMock });
    const missingCas = {
      schema_version: 1,
      theme: "dark",
      default_approval_policy: "ask",
    } as unknown as SettingsPreferencesUpdateRequest;

    await expect(client.updatePreferences(missingCas)).rejects.toBeInstanceOf(
      ProductApiSchemaError,
    );
    await expect(
      client.getMemoryTopic("workspace-1", "../private"),
    ).rejects.toBeInstanceOf(ProductApiSchemaError);
    expect(() => client.listMemoryTopics("\u0000workspace")).toThrow(
      ProductApiSchemaError,
    );
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("surfaces typed CAS conflicts and rejects a false success revision", async () => {
    const conflictClient = createSettingsPlatformClient({
      fetch: vi.fn(async () =>
        jsonResponse(
          {
            code: "product_revision_conflict",
            error: "product preferences changed concurrently",
          },
          409,
        ),
      ),
    });
    const request: SettingsPreferencesUpdateRequest = {
      schema_version: 1,
      expected_revision: 7,
      theme: "dark",
      default_approval_policy: "ask",
    };

    await expect(conflictClient.updatePreferences(request)).rejects.toMatchObject({
      name: "ProductApiError",
      status: 409,
      code: "product_revision_conflict",
    } satisfies Partial<ProductApiError>);

    const falseSuccessClient = createSettingsPlatformClient({
      fetch: vi.fn(async () => jsonResponse(preferences)),
    });
    await expect(
      falseSuccessClient.updatePreferences(request),
    ).rejects.toBeInstanceOf(ProductApiSchemaError);

    const droppedSelectionClient = createSettingsPlatformClient({
      fetch: vi.fn(async () =>
        jsonResponse({
          ...preferences,
          revision: 8,
          provider_selection: undefined,
        }),
      ),
    });
    await expect(
      droppedSelectionClient.updateDefaultApprovalPolicy(preferences, "ask"),
    ).rejects.toBeInstanceOf(ProductApiSchemaError);
  });
});
