import { afterEach, describe, expect, it, vi } from "vitest";

import { ProductApiSchemaError } from "../product/product-api-types";
import { ProductApiError } from "../product/product-client";
import {
  parseCreateProductMcpServerRequest,
  parseProductMcpHealthResponse,
  parseProductMcpProbeResponse,
  parseProductMcpServersResponse,
  parseProductMemoryTopicContentResponse,
  parseProductMemoryTopicsResponse,
  parseProductRuntimeInfo,
  parseProductTrustDecisionRequest,
  parseProductTrustStatus,
  parseSettingsPreferencesUpdateRequest,
  parseUpdateProductMcpServerRequest,
  type SettingsPreferencesUpdateRequest,
} from "./settings-platform-api-types";
import { createSettingsPlatformClient } from "./settings-platform-client";

const topic = {
  slug: "project-conventions",
  title: "Project Conventions",
  layer: "durable",
  memory_type: "project",
  scope: "project",
  source: "product_settings",
  confidence: 0.8,
  created_at: "2026-07-27T00:00:00Z",
  updated_at: "2026-07-27T01:00:00Z",
  description: "project memory",
  metadata_truncated: false,
} as const;

const mcpServer = {
  name: "workspace_tools",
  enabled: true,
  required: true,
  transport: "stdio",
  command: "python",
  args: ["mcp_server.py"],
  env_names: ["WORKSPACE_MCP_TOKEN"],
  request_timeout_ms: 5_000,
  transport_deprecated: false,
} as const;

const mcpHealth = {
  servers: [
    {
      server_name: "workspace_tools",
      required: true,
      transport: "stdio",
      status: "ready",
      server_config_hash:
        "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
      server_identity_hash:
        "sha256:1123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
      protocol_version: "2025-03-26",
      catalog_hash:
        "sha256:2123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
      capability_snapshot_id:
        "sha256:3123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
      tool_count: 2,
      refreshed_at: "2026-08-10T00:00:00Z",
    },
  ],
  total: 1,
} as const;

const mcpProbe = {
  server_name: "workspace_tools",
  transport: "stdio",
  tools: [
    {
      name: "read_workspace",
      description: "Read a workspace file",
      destructive: true,
      parallel_safe: false,
    },
  ],
  tested_at: "2026-08-05T12:00:00Z",
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
  execution_environment: {
    adapter: "local",
    workspace_kind: "repo",
    workspace_digest:
      "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    capabilities: {
      filesystem_read: true,
      filesystem_write: true,
      process_run: true,
      process_stdio: true,
      observations: true,
      process_background: true,
      process_pty: false,
      workspace_checkpoints: true,
      artifact_projection: true,
    },
  },
  agent: {
    selector: "builtin:legacy",
    workspace_source_authorized: false,
    workspace_instructions_enabled: false,
    allow_remediation_procedures: false,
    max_procedure_selections: 3,
  },
  resume_health: {
    status: "healthy",
    workspace_count: 1,
    session_count: 1,
    bound_session_count: 1,
    running_session_count: 0,
    needs_attention_session_count: 0,
  },
} as const;

const trustStatus = {
  workspace_id: "workspace-1",
  state: "trusted",
  identity_digest:
    "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
  invalidated_capabilities: ["mcp_processes"],
  granted_capabilities: ["project_configuration"],
} as const;

function jsonResponse(value: unknown, status = 200): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json" },
  });
}

afterEach(() => {
  vi.unstubAllGlobals();
});

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
        execution_environment: runtimeInfo.execution_environment,
        agent: runtimeInfo.agent,
      }),
    ).toEqual({
      api_version: "0.1.0",
      connection: "connected",
      product_store: "unavailable",
      execution_environment: runtimeInfo.execution_environment,
      agent: runtimeInfo.agent,
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
        ...runtimeInfo,
        execution_environment: {
          ...runtimeInfo.execution_environment,
          workspace_digest: "D:\\private\\workspace",
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

  it("strictly validates secret-free MCP configs and local tool policy", () => {
    expect(
      parseProductMcpServersResponse({ servers: [mcpServer], total: 1 }),
    ).toEqual({ servers: [mcpServer], total: 1 });
    expect(parseProductMcpHealthResponse(mcpHealth)).toEqual(mcpHealth);
    expect(parseProductMcpProbeResponse(mcpProbe)).toEqual(mcpProbe);
    expect(() =>
      parseCreateProductMcpServerRequest({
        ...mcpServer,
        env: { WORKSPACE_MCP_TOKEN: "raw-secret" },
      }),
    ).toThrow(ProductApiSchemaError);
    // A client never declares deprecation: the server owns that verdict, and
    // the API rejects the field as unknown on a create or update request.
    expect(() => parseCreateProductMcpServerRequest({ ...mcpServer })).toThrow(
      ProductApiSchemaError,
    );
    const { name: _name, ...updateRequest } = mcpServer;
    expect(() => parseUpdateProductMcpServerRequest({ ...updateRequest })).toThrow(
      ProductApiSchemaError,
    );
    // A response without the server-owned verdict is not silently defaulted.
    const { transport_deprecated: _omitted, ...withoutVerdict } = mcpServer;
    expect(() =>
      parseProductMcpServersResponse({ servers: [withoutVerdict], total: 1 }),
    ).toThrow(ProductApiSchemaError);
    expect(
      parseProductMcpServersResponse({
        servers: [
          {
            name: mcpServer.name,
            enabled: true,
            required: false,
            transport: "sse",
            args: [],
            env_names: [],
            url: "https://mcp.example.com/sse",
            request_timeout_ms: 5_000,
            transport_deprecated: true,
          },
        ],
        total: 1,
      }).servers[0].transport_deprecated,
    ).toBe(true);
    expect(() =>
      parseCreateProductMcpServerRequest({
        ...mcpServer,
        args: ["--token=raw-secret"],
      }),
    ).toThrow(ProductApiSchemaError);
    expect(() =>
      parseCreateProductMcpServerRequest({
        ...mcpServer,
        name: "unsafe-name",
      }),
    ).toThrow(ProductApiSchemaError);
    expect(() =>
      parseProductMcpProbeResponse({
        ...mcpProbe,
        tools: [{ ...mcpProbe.tools[0], destructive: false }],
      }),
    ).toThrow(ProductApiSchemaError);
    expect(() =>
      parseProductMcpHealthResponse({
        servers: [mcpHealth.servers[0], mcpHealth.servers[0]],
        total: 2,
      }),
    ).toThrow(ProductApiSchemaError);
    expect(() =>
      parseProductMcpHealthResponse({
        servers: [{ ...mcpHealth.servers[0], failure_code: "raw secret\nvalue" }],
        total: 1,
      }),
    ).toThrow(ProductApiSchemaError);
  });

  it("strictly parses project trust states and explicit decisions", () => {
    expect(parseProductTrustStatus(trustStatus)).toEqual(trustStatus);
    expect(
      parseProductTrustDecisionRequest({
        decision: "grant",
        capabilities: ["project_configuration", "mcp_processes"],
      }),
    ).toEqual({
      decision: "grant",
      capabilities: ["project_configuration", "mcp_processes"],
    });
    expect(() =>
      parseProductTrustStatus({
        ...trustStatus,
        granted_capabilities: ["project_configuration", "project_configuration"],
      }),
    ).toThrow(ProductApiSchemaError);
    expect(() =>
      parseProductTrustStatus({ ...trustStatus, canonical_root: "D:/private" }),
    ).toThrow(ProductApiSchemaError);
    expect(() =>
      parseProductTrustStatus({
        ...trustStatus,
        identity_digest: "D:\\private\\workspace",
      }),
    ).toThrow(ProductApiSchemaError);
  });
});

describe("settings platform client", () => {
  it("uses the authenticated Desktop loopback transport", async () => {
    vi.stubGlobal("window", {
      __ROVE_API_URL__: "http://127.0.0.1:49152",
      __ROVE_TOKEN__: "desktop-secret",
    });
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, _init?: RequestInit) =>
        jsonResponse(runtimeInfo),
    );

    await createSettingsPlatformClient({ fetch: fetchMock }).getRuntimeInfo();

    expect(fetchMock).toHaveBeenCalledOnce();
    expect(String(fetchMock.mock.calls[0]?.[0])).toBe(
      "http://127.0.0.1:49152/product/runtime",
    );
    const headers = new Headers(fetchMock.mock.calls[0]?.[1]?.headers);
    expect(headers.get("authorization")).toBe("Bearer desktop-secret");
  });

  it("uses only the server-owned workspace id for trust decisions", async () => {
    const calls: Array<{ url: string; method: string; body?: string }> = [];
    const fetchMock: typeof globalThis.fetch = vi.fn(
      async (input: RequestInfo | URL, init?: RequestInit) => {
        const method = init?.method ?? "GET";
        const body = typeof init?.body === "string" ? init.body : undefined;
        calls.push({ url: String(input), method, body });
        return jsonResponse(
          method === "PUT"
            ? { ...trustStatus, invalidated_capabilities: [], granted_capabilities: ["mcp_processes"] }
            : trustStatus,
        );
      },
    );
    const client = createSettingsPlatformClient({ fetch: fetchMock });

    await client.getProjectTrust("workspace-1");
    await client.decideProjectTrust("workspace-1", {
      decision: "grant",
      capabilities: ["mcp_processes"],
    });

    expect(calls).toEqual([
      {
        url: "/api/product/workspaces/workspace-1/trust",
        method: "GET",
        body: undefined,
      },
      {
        url: "/api/product/workspaces/workspace-1/trust",
        method: "PUT",
        body: JSON.stringify({
          decision: "grant",
          capabilities: ["mcp_processes"],
        }),
      },
    ]);
    expect(calls[1]?.body).not.toContain("root");
    expect(calls[1]?.body).not.toContain("path");
  });

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
          url ===
            "/api/product/memory/topics?workspace_id=workspace-1&q=project+rules&memory_type=project&scope=project&source=product_settings" &&
          method === "GET"
        ) {
          return jsonResponse({ topics: [topic], total: 1 });
        }
        if (
          url === "/api/product/memory/topics?workspace_id=workspace-1" &&
          method === "POST"
        ) {
          const request = JSON.parse(String(init?.body)) as Record<
            string,
            unknown
          >;
          return jsonResponse(
            {
              topic: {
                ...topic,
                slug: request.slug,
                title: request.title,
                memory_type: request.memory_type,
                scope: request.scope,
                confidence: request.confidence,
                description: request.description,
              },
              content: request.content,
              truncated: false,
            },
            201,
          );
        }
        if (
          url ===
            "/api/product/memory/topics/project-conventions?workspace_id=workspace-1" &&
          method === "PUT"
        ) {
          const request = JSON.parse(String(init?.body)) as Record<
            string,
            unknown
          >;
          return jsonResponse({
            topic: {
              ...topic,
              title: request.title,
              memory_type: request.memory_type,
              scope: request.scope,
              confidence: request.confidence,
              description: request.description,
              updated_at: "2026-07-27T02:00:00Z",
            },
            content: request.content,
            truncated: false,
          });
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

    await client.listMemoryTopics("workspace-1", {
      q: " project rules ",
      memory_type: "project",
      scope: "project",
      source: "product_settings",
    });
    await client.createMemoryTopic("workspace-1", {
      slug: "created-topic",
      title: "Created Topic",
      memory_type: "reference",
      scope: "global",
      confidence: 0.9,
      description: "Created in Settings",
      content: "Created durable fact.\n",
    });
    await client.updateMemoryTopic("workspace-1", "project-conventions", {
      title: "Updated Conventions",
      memory_type: "project",
      scope: "project",
      confidence: 0.95,
      description: "Updated in Settings",
      content: "Updated durable fact.\n",
      expected_updated_at: topic.updated_at,
    });
    await client.getMemoryTopic("workspace-1", "project-conventions");
    await client.deleteMemoryTopic("workspace-1", "project-conventions");
    await client.getRuntimeInfo();
    const current = await client.getPreferences();
    const updated = await client.updateDefaultApprovalPolicy(current, "never");

    expect(updated.default_approval_policy).toBe("never");
    expect(updated.provider_selection?.approval).toBe("never");
    expect(calls.map(({ url, method }) => `${method} ${url}`)).toEqual([
      "GET /api/product/memory/topics?workspace_id=workspace-1&q=project+rules&memory_type=project&scope=project&source=product_settings",
      "POST /api/product/memory/topics?workspace_id=workspace-1",
      "PUT /api/product/memory/topics/project-conventions?workspace_id=workspace-1",
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
    await expect(
      client.createMemoryTopic("workspace-1", {
        slug: "unsafe-title",
        title: "Unsafe](topics/escape.md)",
        memory_type: "project",
        scope: "project",
        confidence: 0.8,
        description: "must not send",
        content: "content",
      }),
    ).rejects.toBeInstanceOf(ProductApiSchemaError);
    expect(() =>
      client.listMemoryTopics("workspace-1", { q: "line\nbreak" }),
    ).toThrow(ProductApiSchemaError);
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

  it("covers workspace MCP CRUD and probe without transmitting environment values", async () => {
    const calls: Array<{ url: string; method: string; body?: string }> = [];
    const fetchMock: typeof globalThis.fetch = vi.fn(
      async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        const method = init?.method ?? "GET";
        const body = typeof init?.body === "string" ? init.body : undefined;
        calls.push({ url, method, body });
        if (
          url === "/api/product/mcp/servers?workspace_id=workspace-1" &&
          method === "GET"
        ) {
          return jsonResponse({ servers: [mcpServer], total: 1 });
        }
        if (
          url === "/api/product/mcp/health?workspace_id=workspace-1" &&
          method === "GET"
        ) {
          return jsonResponse(mcpHealth);
        }
        if (
          url === "/api/product/mcp/servers?workspace_id=workspace-1" &&
          method === "POST"
        ) {
          // The real server always returns its own deprecation verdict.
          return jsonResponse(
            { transport_deprecated: false, ...JSON.parse(body ?? "{}") },
            201,
          );
        }
        if (
          url ===
            "/api/product/mcp/servers/workspace_tools?workspace_id=workspace-1" &&
          method === "PUT"
        ) {
          return jsonResponse({
            name: "workspace_tools",
            transport_deprecated: false,
            ...JSON.parse(body ?? "{}"),
          });
        }
        if (
          url ===
            "/api/product/mcp/servers/workspace_tools/probe?workspace_id=workspace-1" &&
          method === "POST"
        ) {
          return jsonResponse(mcpProbe);
        }
        if (
          url ===
            "/api/product/mcp/servers/workspace_tools?workspace_id=workspace-1" &&
          method === "DELETE"
        ) {
          return new Response(null, { status: 204 });
        }
        return jsonResponse({ code: "not_found", error: "unexpected route" }, 404);
      },
    );
    const client = createSettingsPlatformClient({ fetch: fetchMock });

    await client.listMcpServers("workspace-1");
    await expect(client.getMcpHealth("workspace-1")).resolves.toEqual(mcpHealth);
    const { transport_deprecated: _serverOwned, ...createRequest } = mcpServer;
    await client.createMcpServer("workspace-1", {
      ...createRequest,
      args: [...mcpServer.args],
      env_names: [...mcpServer.env_names],
    });
    await client.updateMcpServer("workspace-1", "workspace_tools", {
      enabled: false,
      required: false,
      transport: "stdio",
      command: "python",
      args: ["mcp_server.py"],
      env_names: ["WORKSPACE_MCP_TOKEN"],
      request_timeout_ms: 9_000,
    });
    await client.probeMcpServer("workspace-1", "workspace_tools");
    await client.deleteMcpServer("workspace-1", "workspace_tools");

    expect(calls.map(({ method, url }) => `${method} ${url}`)).toEqual([
      "GET /api/product/mcp/servers?workspace_id=workspace-1",
      "GET /api/product/mcp/health?workspace_id=workspace-1",
      "POST /api/product/mcp/servers?workspace_id=workspace-1",
      "PUT /api/product/mcp/servers/workspace_tools?workspace_id=workspace-1",
      "POST /api/product/mcp/servers/workspace_tools/probe?workspace_id=workspace-1",
      "DELETE /api/product/mcp/servers/workspace_tools?workspace_id=workspace-1",
    ]);
    const serializedBodies = calls
      .map((call) => call.body ?? "")
      .join("\n");
    expect(serializedBodies).toContain('"env_names":["WORKSPACE_MCP_TOKEN"]');
    expect(serializedBodies).not.toContain('"env"');
    expect(serializedBodies).not.toContain("raw-secret");

    const unsafeClient = createSettingsPlatformClient({ fetch: vi.fn() });
    await expect(
      unsafeClient.createMcpServer(
        "workspace-1",
        {
          ...mcpServer,
          env: { WORKSPACE_MCP_TOKEN: "raw-secret" },
        } as never,
      ),
    ).rejects.toBeInstanceOf(ProductApiSchemaError);
  });

  it("preserves typed MCP probe failures", async () => {
    const client = createSettingsPlatformClient({
      fetch: vi.fn(async () =>
        jsonResponse(
          { code: "product_mcp_timeout", error: "the MCP probe timed out" },
          504,
        ),
      ),
    });
    await expect(
      client.probeMcpServer("workspace-1", "workspace_tools"),
    ).rejects.toMatchObject({
      status: 504,
      code: "product_mcp_timeout",
    } satisfies Partial<ProductApiError>);
  });
});
