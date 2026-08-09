import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { ProductApiError } from "../product/product-client";
import {
  MCPSettings,
  createEmptyMcpServerDraft,
  describeMcpProbeFailure,
  mcpServerDraftFromConfig,
  mcpServerRequestFromDraft,
} from "./MCPSettings";
import { createSettingsPlatformClient } from "./settings-platform-client";

describe("MCPSettings", () => {
  it("renders workspace-scoped MCP controls without a raw environment value field", () => {
    const client = createSettingsPlatformClient({
      fetch: vi.fn() as unknown as typeof globalThis.fetch,
    });
    const html = renderToStaticMarkup(
      <MCPSettings client={client} workspaceId="workspace-1" />,
    );

    expect(html).toContain('for="mcp-server-name"');
    expect(html).toContain('for="mcp-command"');
    expect(html).toContain('for="mcp-env-names"');
    expect(html).toContain("Legacy SSE");
    expect(html).toContain("Loading MCP servers");
    expect(html).not.toContain('name="env"');
    expect(html).not.toContain("Environment value");
  });

  it("round-trips an editable stdio draft and strips stdio fields from SSE", () => {
    const draft = mcpServerDraftFromConfig({
      name: "workspace_tools",
      enabled: false,
      transport: "stdio",
      command: "python",
      args: ["server.py", "--verbose"],
      env_names: ["MCP_TOKEN", "MCP_REGION"],
      request_timeout_ms: 9_000,
      transport_deprecated: false,
    });
    expect(draft).toMatchObject({
      name: "workspace_tools",
      enabled: false,
      argsText: "server.py\n--verbose",
      envNamesText: "MCP_TOKEN\nMCP_REGION",
      timeoutMs: "9000",
    });
    expect(mcpServerRequestFromDraft(draft)).toEqual({
      name: "workspace_tools",
      enabled: false,
      transport: "stdio",
      command: "python",
      args: ["server.py", "--verbose"],
      env_names: ["MCP_TOKEN", "MCP_REGION"],
      request_timeout_ms: 9_000,
    });

    expect(
      mcpServerRequestFromDraft({
        ...createEmptyMcpServerDraft(),
        name: "legacy_sse",
        transport: "sse",
        command: "must-not-send",
        argsText: "--token=must-not-send",
        envNamesText: "MUST_NOT_SEND",
        url: "http://127.0.0.1:3001/sse",
      }),
    ).toEqual({
      name: "legacy_sse",
      enabled: true,
      transport: "sse",
      args: [],
      env_names: [],
      url: "http://127.0.0.1:3001/sse",
      request_timeout_ms: 30_000,
    });

    expect(
      mcpServerRequestFromDraft({
        ...createEmptyMcpServerDraft(),
        name: "streaming",
        transport: "streamable_http",
        command: "must-not-send",
        argsText: "--token=must-not-send",
        envNamesText: "MUST_NOT_SEND",
        url: "https://mcp.example.com/mcp ",
      }),
    ).toEqual({
      name: "streaming",
      enabled: true,
      transport: "streamable_http",
      args: [],
      env_names: [],
      url: "https://mcp.example.com/mcp",
      request_timeout_ms: 30_000,
    });
  });

  it("maps typed probe failures to actionable messages", () => {
    expect(
      describeMcpProbeFailure(
        new ProductApiError(504, "product_mcp_timeout", "generic"),
      ),
    ).toContain("timed out");
    expect(
      describeMcpProbeFailure(
        new ProductApiError(
          502,
          "product_mcp_protocol_mismatch",
          "generic",
        ),
      ),
    ).toContain("compatible MCP tool catalog");
    expect(
      describeMcpProbeFailure(
        new ProductApiError(502, "product_mcp_no_tools", "generic"),
      ),
    ).toContain("returned no tools");
  });
});
