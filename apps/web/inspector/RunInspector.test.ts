import { describe, expect, it } from "vitest";

import { createWorkbenchState } from "../lib/rove-state";
import { resolveInspectorPhase } from "./RunInspector";

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
});
