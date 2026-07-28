import { renderToStaticMarkup } from "react-dom/server";
import { createElement } from "react";
import { describe, expect, it, vi } from "vitest";

import { createWorkbenchState } from "../lib/rove-state";
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

  it("renders exact lifecycle identities and opaque evidence refs without fake actions", () => {
    const state = {
      ...createWorkbenchState(),
      activeJobId: "job-01JEXACTIDENTITY000000000001",
      activeRunId: "run-01JEXACTIDENTITY000000000001",
      activeRunOrdinal: 3,
      resumedFromRunId: "run-01JPREVIOUS000000000000001",
      eventCount: 1,
      statusText: "Run completed",
      stepRecords: [
        {
          record_id: "record-1",
          plan_id: "plan-1",
          plan_revision_id: "revision-1",
          step_id: "step-1",
          attempt: 1,
          status: "succeeded" as const,
          started_at: "2026-07-28T00:00:00Z",
          finished_at: "2026-07-28T00:00:01Z",
          summary: "Evidence retained",
          completion_basis: "model_conclusion" as const,
          evidence_refs: ["trace:42"],
          artifact_refs: ["artifact:opaque-7"],
          model_turns_used: 1,
          tool_calls_used: 1,
          token_usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
        },
      ],
    };

    const html = renderToStaticMarkup(
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
    );

    expect(html).toContain("product-session-01JEXACT000000000001");
    expect(html).toContain("job-01JEXACTIDENTITY000000000001");
    expect(html).toContain("run-01JEXACTIDENTITY000000000001");
    expect(html).toContain("artifact:opaque-7");
    expect(html).toContain("trace:42");
    expect(html).toContain("Cost requires a trusted server pricing snapshot");
    expect(html).not.toContain("Download artifact");
    expect(html).not.toContain("Open artifact");
  });
});
