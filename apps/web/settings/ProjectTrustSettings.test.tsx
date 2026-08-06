import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { ProjectTrustSettings } from "./ProjectTrustSettings";
import { createSettingsPlatformClient } from "./settings-platform-client";

describe("ProjectTrustSettings", () => {
  it("renders granular controls without exposing a local path input", () => {
    const client = createSettingsPlatformClient({
      fetch: vi.fn() as unknown as typeof globalThis.fetch,
    });
    const html = renderToStaticMarkup(
      <ProjectTrustSettings client={client} workspaceId="workspace-1" />,
    );

    expect(html).toContain("Project trust");
    expect(html).toContain("Project configuration");
    expect(html).toContain("MCP processes");
    expect(html).toContain("Grant selected");
    expect(html).toContain("Deny");
    expect(html).toContain("Revoke");
    expect(html).not.toContain('type="text"');
    expect(html).not.toContain("canonical_root");
  });
});
