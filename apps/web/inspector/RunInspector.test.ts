import { renderToStaticMarkup } from "react-dom/server";
import { createElement } from "react";
import { describe, expect, it, vi } from "vitest";

import { CopyProvider } from "../copy/CopyProvider";
import { createWorkbenchState, isTerminalRunCanceled } from "../lib/rove-state";
import { RunInspector, resolveInspectorPhase } from "./RunInspector";

describe("resolveInspectorPhase", () => {
  it("returns empty when no run has started", () => {
    expect(resolveInspectorPhase(createWorkbenchState())).toBe("empty");
  });

  it("returns loading while a run is busy", () => {
    const state = {
      ...createWorkbenchState(),
      busy: true,
      activeJobId: "job-1",
      statusText: "Streaming run events",
    };
    expect(resolveInspectorPhase(state)).toBe("loading");
  });

  it("returns error when a hard or soft run error is present", () => {
    const state = {
      ...createWorkbenchState(),
      error: "Hard resume failed: missing checkpoint",
      statusText: "Run interrupted",
    };
    expect(resolveInspectorPhase(state)).toBe("error");
  });

  it("returns ready after a completed run leaves identity/content", () => {
    const state = {
      ...createWorkbenchState(),
      activeJobId: "job-1",
      activeRunId: "run-1",
      eventCount: 4,
      statusText: "Run completed",
    };
    expect(resolveInspectorPhase(state)).toBe("ready");
  });

  it("keeps internal identifiers out of the product chrome", () => {
    const state = {
      ...createWorkbenchState(),
      activeJobId: "job-01JEXACTIDENTITY000000000001",
      activeRunId: "run-01JEXACTIDENTITY000000000001",
      activeRunOrdinal: 3,
      resumedFromRunId: "run-01JPREVIOUS000000000000001",
      eventCount: 1,
      statusText: "Run completed",
      runUsage: {
        prompt_tokens: 100,
        completion_tokens: 50,
        total_tokens: 150,
      },
      tools: [
        {
          id: "call-1",
          name: "read_file",
          status: "done" as const,
          details: "read src/app.ts",
        },
      ],
    };

    const html = renderToStaticMarkup(
      createElement(
        CopyProvider,
        null,
        createElement(RunInspector, {
          productSessionId: "product-session-01JEXACT000000000001",
          collapsed: false,
          onToggle: vi.fn(),
          runState: state,
          restoreState: {
            status: "complete",
            sessionId: "product-session-01JEXACT000000000001",
          },
        }),
      ),
    );

    expect(html).not.toContain("product-session-01JEXACT");
    expect(html).not.toContain("job-01JEXACTIDENTITY");
    expect(html).not.toContain("run-01JEXACTIDENTITY");
    expect(html).not.toContain("prompt hash");
    expect(html).not.toContain("cache key");
    expect(html).not.toContain("canonical");
    expect(html).not.toContain("risk");
    expect(html).toContain("read_file");
    expect(html).toContain("本次运行");
  });
  it("labels canceled terminal runs without implying completion", () => {
    expect(
      isTerminalRunCanceled({
        statusText: "Run interrupted",
        busy: false,
        error: null,
      }),
    ).toBe(true);
    const state = {
      ...createWorkbenchState(),
      activeJobId: "job-1",
      activeRunId: "run-1",
      eventCount: 4,
      statusText: "Run cancelled",
    };
    expect(resolveInspectorPhase(state)).toBe("ready");
    const html = renderToStaticMarkup(
      createElement(
        CopyProvider,
        null,
        createElement(RunInspector, {
          productSessionId: "product-session-01JCANCEL000000000001",
          collapsed: false,
          onToggle: vi.fn(),
          runState: state,
          restoreState: {
            status: "complete",
            sessionId: "product-session-01JCANCEL000000000001",
          },
        }),
      ),
    );
    expect(html).toContain("已取消");
    expect(html).not.toContain("已完成");
    expect(html).not.toContain("运行完成");
  });
});
