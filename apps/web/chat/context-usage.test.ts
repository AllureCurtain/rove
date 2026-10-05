import { describe, expect, it } from "vitest";

import { contextUsage, latestUsage } from "./context-usage";

describe("contextUsage", () => {
  it("stays neutral and ratio-free when the window is unknown", () => {
    const usage = contextUsage({ promptTokens: 100, completionTokens: 50, window: null });
    expect(usage.ratio).toBeNull();
    expect(usage.tone).toBe("neutral");
    expect(usage.used).toBe(150);
  });

  it("warns at 25% remaining and turns danger at 10%", () => {
    const warn = contextUsage({ promptTokens: 750, completionTokens: 0, window: 1000 });
    const danger = contextUsage({ promptTokens: 900, completionTokens: 0, window: 1000 });
    const fine = contextUsage({ promptTokens: 740, completionTokens: 0, window: 1000 });
    expect(warn.tone).toBe("warn");
    expect(danger.tone).toBe("danger");
    expect(fine.tone).toBe("neutral");
  });

  it("omits the cache ratio when there is no input to measure", () => {
    const empty = contextUsage({ promptTokens: 0, completionTokens: 0, window: 1000 });
    expect(empty.cacheRatio).toBeNull();
    const cached = contextUsage({ promptTokens: 50, completionTokens: 0, cachedTokens: 50, window: 1000 });
    expect(cached.cacheRatio).toBe(0.5);
  });
});

describe("latestUsage", () => {
  it("skips a trailing entry that has no usage", () => {
    const entries = [{ id: "a", usage: true }, { id: "b", usage: false }];
    const found = latestUsage(entries, (entry) => entry.usage);
    expect(found?.id).toBe("a");
  });

  it("returns null when nothing carries usage", () => {
    expect(latestUsage([{ usage: false }], (entry) => entry.usage)).toBeNull();
  });
});
