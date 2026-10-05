import { describe, expect, it } from "vitest";

import { filterSessions, isCurrentSearch, nextSearchToken, sessionMatchesQuery } from "./session-search";

const sessions = [{ title: "修复布局" }, { title: "Add tests" }];

describe("session search", () => {
  it("matches a substring and not a subsequence", () => {
    expect(sessionMatchesQuery("修复布局", "布局")).toBe(true);
    expect(sessionMatchesQuery("修复布局", "修布")).toBe(false);
  });

  it("is case-insensitive", () => {
    expect(sessionMatchesQuery("Add tests", "ADD")).toBe(true);
  });

  it("restores the full list for an empty query instead of reporting no results", () => {
    const result = filterSessions(sessions, "   ");
    expect(result.empty).toBe(false);
    expect(result.matches).toHaveLength(2);
  });

  it("reports an empty result when nothing matches", () => {
    expect(filterSessions(sessions, "不存在").empty).toBe(true);
  });

  it("invalidates an older search token", () => {
    const first = nextSearchToken(0);
    const second = nextSearchToken(first);
    expect(isCurrentSearch(first, second)).toBe(false);
    expect(isCurrentSearch(second, second)).toBe(true);
  });
});
