import { describe, expect, it } from "vitest";

import type { ProductSearchHit } from "../product/product-api-types";
import {
  SEARCH_PAGE_LIMIT,
  describeSearchHit,
  expiredRecovery,
  formatSearchScope,
  groupHitsBySession,
  mergeSearchPage,
  nextSearchPageRequest,
  parseSearchScope,
  searchPageRequest,
  searchPartial,
  searchScopeOptions,
  searchTermBytes,
  workspaceScopeTermOk,
} from "./product-search-model";

function hit(overrides: Partial<ProductSearchHit> = {}): ProductSearchHit {
  return {
    session_id: "s1",
    source: "message",
    seq: 4,
    snippet: "… needle …",
    created_at: "2026-09-26T00:00:00.000Z",
    ...overrides,
  };
}

describe("search scopes", () => {
  it("round-trips a canonical scope", () => {
    const scope = { kind: "workspace" as const, id: "w1" };
    expect(formatSearchScope(scope)).toBe("workspace:w1");
    expect(parseSearchScope("workspace:w1")).toEqual(scope);
  });

  it("refuses a scope the server would not resolve", () => {
    for (const value of [
      "",
      "workspace",
      "unknown:w1",
      "workspace:",
      "workspace:a b",
      "workspace:a:b",
      "workspace:a\u0000b",
    ]) {
      expect(parseSearchScope(value), value).toBeNull();
    }
  });

  it("marks a scope unavailable with its own reason", () => {
    const options = searchScopeOptions({ sessionId: null, workspaceId: "w1" });
    expect(options.find((option) => option.kind === "session")).toEqual({
      kind: "session",
      available: false,
      reasonCopyKey: "search.scopeSessionUnavailable",
    });
    expect(options.find((option) => option.kind === "trace")?.reasonCopyKey).toBe(
      "search.scopeTraceUnavailable",
    );
    expect(options.find((option) => option.kind === "workspace")?.available).toBe(true);
  });
});

describe("term rules", () => {
  it("refuses a workspace term shorter than three characters", () => {
    expect(workspaceScopeTermOk("ab")).toBe(false);
    expect(workspaceScopeTermOk("abc")).toBe(true);
  });

  it("counts a term in UTF-8 bytes, not characters", () => {
    expect(searchTermBytes("abc")).toBe(3);
    // Three CJK characters are nine UTF-8 bytes.
    expect(searchTermBytes("搜索词")).toBe(9);
  });
});

describe("page requests", () => {
  const scope = { kind: "session" as const, id: "s1" };

  it("starts without a cursor", () => {
    expect(searchPageRequest("needle", scope)).toEqual({
      q: "needle",
      scope: "session:s1",
      limit: SEARCH_PAGE_LIMIT,
    });
  });

  it("only reuses a cursor with the same scope the page echoed", () => {
    expect(
      nextSearchPageRequest(
        { term: "needle", scope },
        { scope: "session:s1", next_cursor: "c1" },
      ),
    ).toEqual({ q: "needle", scope: "session:s1", cursor: "c1", limit: SEARCH_PAGE_LIMIT });
  });

  it("drops the cursor when the server echoes another scope", () => {
    expect(
      nextSearchPageRequest(
        { term: "needle", scope },
        { scope: "workspace:w1", next_cursor: "c1" },
      ),
    ).toBeNull();
  });

  it("has no next page without a cursor", () => {
    expect(
      nextSearchPageRequest({ term: "needle", scope }, { scope: "session:s1" }),
    ).toBeNull();
  });
});

describe("partial pages", () => {
  it("reports a short page that still has a cursor", () => {
    expect(searchPartial({ hitCount: 3, limit: 32, nextCursor: "c1" })).toBe(true);
  });

  it("does not report a short page without a cursor as partial", () => {
    expect(searchPartial({ hitCount: 3, limit: 32 })).toBe(false);
  });

  it("does not report a full page as partial on its own", () => {
    expect(searchPartial({ hitCount: 32, limit: 32, nextCursor: "c1" })).toBe(false);
  });
});

describe("hit facts", () => {
  it("keeps a trace hit's run ordinal and reports a missing timestamp", () => {
    const facts = describeSearchHit(
      hit({ source: "trace", run_ordinal: 2, created_at: undefined }),
    );
    expect(facts.sourceCopyKey).toBe("search.hitTrace");
    expect(facts.runOrdinal).toBe(2);
    expect(facts.hasCreatedAt).toBe(false);
    expect(facts.locatable).toBe(false);
  });

  it("does not invent a run ordinal", () => {
    expect(describeSearchHit(hit({ source: "trace" })).runOrdinal).toBeNull();
  });

  it("only a message hit is locatable by its sequence", () => {
    expect(describeSearchHit(hit()).locatable).toBe(true);
  });
});

describe("grouping and merging", () => {
  it("groups workspace hits by session in first-seen order", () => {
    const groups = groupHitsBySession([
      hit({ session_id: "s2", seq: 1 }),
      hit({ session_id: "s1", seq: 2 }),
      hit({ session_id: "s2", seq: 3 }),
    ]);
    expect(groups.map((group) => group.sessionId)).toEqual(["s2", "s1"]);
    expect(groups[0]!.hits.map((item) => item.seq)).toEqual([1, 3]);
  });

  it("appends a page from the same scope", () => {
    const merged = mergeSearchPage(
      { scope: { kind: "session", id: "s1" }, hits: [hit({ seq: 1 })] },
      { scope: "session:s1", hits: [hit({ seq: 2 })] },
    );
    expect(merged?.map((item) => item.seq)).toEqual([1, 2]);
  });

  it("refuses to append a page from another scope", () => {
    expect(
      mergeSearchPage(
        { scope: { kind: "session", id: "s1" }, hits: [hit({ seq: 1 })] },
        { scope: "workspace:w1", hits: [hit({ seq: 2 })] },
      ),
    ).toBeNull();
  });
});

describe("expired cursor recovery", () => {
  it("restarts from the first page when a cursor expired", () => {
    expect(expiredRecovery({ hadCursor: true })).toBe("restart");
  });

  it("reports instead of looping when the first page itself expired", () => {
    expect(expiredRecovery({ hadCursor: false })).toBe("report");
  });
});
