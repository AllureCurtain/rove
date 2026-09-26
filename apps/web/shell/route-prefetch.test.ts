import { describe, expect, it } from "vitest";

import {
  ROUTE_PREFETCH_FALLBACK_DELAY_MS,
  ROUTE_PREFETCH_IDLE_TIMEOUT_MS,
  routePrefetchTargets,
  shouldPrefetchRoutes,
} from "./route-prefetch";

describe("idle route prefetch targets", () => {
  it("prefetches the settings sections the header can reach in one click", () => {
    const targets = routePrefetchTargets({ activeWorkspaceId: null });
    expect(targets).toEqual(["/settings/general", "/settings/providers"]);
  });

  it("adds the active workspace as the way back, without duplicates", () => {
    const targets = routePrefetchTargets({ activeWorkspaceId: "ws-1" });
    expect(targets).toContain("/w/ws-1");
    expect(new Set(targets).size).toBe(targets.length);
  });

  it("stays bounded and never prefetches the catalog", () => {
    const targets = routePrefetchTargets({ activeWorkspaceId: "ws-1" });
    expect(targets.length).toBeLessThanOrEqual(4);
    for (const target of targets) {
      expect(target.startsWith("/")).toBe(true);
      expect(target).not.toContain("//");
    }
  });

  it("keeps the reference's scheduling constants", () => {
    // Ported verbatim: an idle timeout, and a timer fallback where
    // requestIdleCallback does not exist.
    expect(ROUTE_PREFETCH_IDLE_TIMEOUT_MS).toBe(8000);
    expect(ROUTE_PREFETCH_FALLBACK_DELAY_MS).toBe(3000);
  });

  it("only prefetches in a production build", () => {
    // Next does not prefetch in dev, and a dev prefetch would only compete with
    // on-demand compilation, so the hook stands down rather than waiting for the
    // framework to ignore it.
    expect(shouldPrefetchRoutes("production")).toBe(true);
    expect(shouldPrefetchRoutes("development")).toBe(false);
    expect(shouldPrefetchRoutes("test")).toBe(false);
    expect(shouldPrefetchRoutes(undefined)).toBe(false);
  });
});
