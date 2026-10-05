import type { ProductMessagesSearchResponse } from "../product/product-api-types";
import { MAX_SEARCH_TERM_BYTES, SEARCH_PAGE_LIMIT } from "./product-search-model";

/**
 * Rules for the session-scoped message search (R7 step 1).
 *
 * The endpoint binds a cursor to the term that produced it, so term and cursor
 * are only ever used as the pair they were issued for. Zero hits is *only* ever
 * a successful empty page: a 400 refusal and a 404 missing session are different
 * answers and must never be rendered as "no results".
 */

export const SESSION_SEARCH_PAGE_LIMIT = SEARCH_PAGE_LIMIT;

/** The client refuses a shorter window; the endpoint has no sub-character term. */
export const MIN_SESSION_SEARCH_TERM_CHARS = 1;

/**
 * Why a term cannot be searched at all, as a copy key, or null when it can.
 *
 * The endpoint answers a blank, over-long or control-bearing term with 400, so
 * the refusal is made before the request rather than presented as an empty
 * result.
 */
export function sessionSearchTermError(term: string): string | null {
  const trimmed = term.trim();
  if (trimmed.length < MIN_SESSION_SEARCH_TERM_CHARS) {
    return "commandPalette.searchTooShort";
  }
  if (new TextEncoder().encode(trimmed).length > MAX_SEARCH_TERM_BYTES) {
    return "search.tooLong";
  }
  if (/[\u0000-\u001f\u007f-\u009f]/u.test(trimmed)) {
    return "commandPalette.searchInvalidInput";
  }
  return null;
}

export interface SessionSearchPage {
  hits: ProductMessagesSearchResponse["hits"];
  nextCursor: string | null;
}

/**
 * The next-page request for an *unchanged* term, or null when there is no next
 * page.
 *
 * The cursor carries a digest of the term it was issued for, so a changed term
 * must never reuse it: the endpoint would answer 400. Returning null rather than
 * a request for the old term is what makes that impossible to get wrong.
 */
export function nextSessionSearchPage(input: {
  currentTerm: string;
  issuedTerm: string;
  nextCursor: string | null;
  limit?: number;
}): { q: string; cursor: string; limit: number } | null {
  if (input.nextCursor === null) {
    return null;
  }
  if (input.currentTerm !== input.issuedTerm) {
    return null;
  }
  return {
    q: input.issuedTerm,
    cursor: input.nextCursor,
    limit: input.limit ?? SESSION_SEARCH_PAGE_LIMIT,
  };
}

/** The three answers the endpoint can give, kept apart on purpose. */
export type SessionSearchFailure =
  | { kind: "refused" }
  | { kind: "not-found" }
  | { kind: "failed"; code: string | null };

/**
 * Classify a failed request. `product_invalid_input` is a refusal of what was
 * asked (usually a cursor that no longer matches the term), and
 * `product_not_found` says the session is gone; neither is "no results".
 */
export function classifySessionSearchFailure(code: string | null): SessionSearchFailure {
  if (code === "product_invalid_input") {
    return { kind: "refused" };
  }
  if (code === "product_not_found") {
    return { kind: "not-found" };
  }
  return { kind: "failed", code };
}
