import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import {
  MemorySettings,
  createEmptyMemoryTopicDraft,
  memoryTopicDraftFromDetail,
} from "./MemorySettings";
import { createSettingsPlatformClient } from "./settings-platform-client";

describe("MemorySettings", () => {
  it("renders durable search, filters, and the create command", () => {
    const client = createSettingsPlatformClient({
      fetch: vi.fn() as unknown as typeof globalThis.fetch,
    });

    const html = renderToStaticMarkup(
      <MemorySettings client={client} workspaceId="workspace-1" />,
    );

    expect(html).toContain('for="memory-search"');
    expect(html).toContain('for="memory-scope-filter">Durable scope');
    expect(html).toContain("New topic");
    expect(html).toContain("Loading durable memory topics");
  });

  it("builds an edit draft from a complete durable topic without losing CAS", () => {
    const draft = memoryTopicDraftFromDetail({
      topic: {
        slug: "session-reference",
        title: "Session Reference",
        layer: "durable",
        memory_type: "reference",
        scope: "session",
        source: "llm_tool",
        confidence: 0.9,
        created_at: "2026-07-26T00:00:00Z",
        updated_at: "2026-07-27T00:00:00Z",
        description: "A durable topic with session scope",
        metadata_truncated: false,
      },
      content: "Retain this exact body.",
      truncated: false,
    });

    expect(draft).toMatchObject({
      slug: "session-reference",
      memoryType: "reference",
      scope: "session",
      content: "Retain this exact body.",
      expectedUpdatedAt: "2026-07-27T00:00:00Z",
    });
    expect(createEmptyMemoryTopicDraft()).toMatchObject({
      memoryType: "project",
      scope: "project",
      confidence: "0.8",
    });
  });
});
