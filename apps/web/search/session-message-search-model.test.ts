import { describe, expect, it } from "vitest";

import {
  SESSION_SEARCH_PAGE_LIMIT,
  classifySessionSearchFailure,
  nextSessionSearchPage,
  sessionSearchTermError,
} from "./session-message-search-model";

describe("sessionSearchTermError", () => {
  it("accepts a single character and a normal term", () => {
    // One character is slow (bounded LIKE) but valid, so it is not refused.
    expect(sessionSearchTermError("a")).toBeNull();
    expect(sessionSearchTermError("needle")).toBeNull();
  });

  it("refuses a blank term and a term the endpoint would answer with 400", () => {
    expect(sessionSearchTermError("   ")).toBe("commandPalette.searchTooShort");
    expect(sessionSearchTermError("")).toBe("commandPalette.searchTooShort");
    expect(sessionSearchTermError("a".repeat(129))).toBe("search.tooLong");
    expect(sessionSearchTermError("a\u0000b")).toBe("commandPalette.searchInvalidInput");
    expect(sessionSearchTermError("a\u007fb")).toBe("commandPalette.searchInvalidInput");
  });

  it("measures the limit in UTF-8 bytes, not characters", () => {
    // 43 three-byte characters are 129 bytes: over the cap although the code
    // point count is far below it.
    expect(sessionSearchTermError("测".repeat(43))).toBe("search.tooLong");
    expect(sessionSearchTermError("测".repeat(42))).toBeNull();
  });
});

describe("nextSessionSearchPage", () => {
  it("never reuses a cursor with a different term", () => {
    // The cursor carries a digest of the term it was issued for; sending it
    // alongside a new term is the 400 the endpoint answers with.
    expect(
      nextSessionSearchPage({
        currentTerm: "changed",
        issuedTerm: "needle",
        nextCursor: "c1",
      }),
    ).toBeNull();
  });

  it("builds the next page from the same term and cursor", () => {
    expect(
      nextSessionSearchPage({
        currentTerm: "needle",
        issuedTerm: "needle",
        nextCursor: "c1",
      }),
    ).toEqual({ q: "needle", cursor: "c1", limit: SESSION_SEARCH_PAGE_LIMIT });
  });

  it("offers no next page when the response carried no cursor", () => {
    expect(
      nextSessionSearchPage({
        currentTerm: "needle",
        issuedTerm: "needle",
        nextCursor: null,
      }),
    ).toBeNull();
  });
});

describe("classifySessionSearchFailure", () => {
  it("keeps a refusal and a missing session apart from an empty result", () => {
    expect(classifySessionSearchFailure("product_invalid_input")).toEqual({
      kind: "refused",
    });
    expect(classifySessionSearchFailure("product_not_found")).toEqual({
      kind: "not-found",
    });
  });

  it("falls back to a typed failure without inventing a cause", () => {
    expect(classifySessionSearchFailure("product_internal_error")).toEqual({
      kind: "failed",
      code: "product_internal_error",
    });
    expect(classifySessionSearchFailure(null)).toEqual({ kind: "failed", code: null });
  });
});
