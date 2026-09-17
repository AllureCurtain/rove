import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { CopyProvider } from "../copy/CopyProvider";
import { createTranslator } from "../copy";
import {
  MemorySettings,
  createEmptyMemoryTopicDraft,
  memoryTopicDraftFromDetail,
} from "./MemorySettings";
import { createSettingsPlatformClient } from "./settings-platform-client";

describe("MemorySettings", () => {
  it("provides the accessible edit command in both languages", () => {
    expect(createTranslator("zh-CN")("memory.editTopic")).toBe("编辑主题");
    expect(createTranslator("en-US")("memory.editTopic")).toBe("Edit topic");
  });
  it("renders durable search, filters, and the create command", () => {
    const client = createSettingsPlatformClient({
      fetch: vi.fn() as unknown as typeof globalThis.fetch,
    });

    const html = renderToStaticMarkup(
      <CopyProvider>
        <MemorySettings client={client} workspaceId="workspace-1" />
      </CopyProvider>,
    );

    expect(html).toContain('for="memory-search"');
    expect(html).toContain('for="memory-scope-filter">作用域');
    expect(html).toContain("新建持久主题");
    expect(html).toContain("正在加载持久记忆主题");
    expect(html).toContain("记忆");
    expect(html).toContain('for="memory-type-filter">类型');
    expect(html).toContain('for="memory-source-filter">来源');
    expect(html).toContain('value="reference">参考');
    expect(html).toContain('value="session">会话');
    expect(html).toContain('value="llm_tool">助手工具');
    expect(html).not.toContain("All types");
    expect(html).not.toContain("memory.editTopic");
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
