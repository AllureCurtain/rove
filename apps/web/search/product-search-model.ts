import type { ProductSearchHit } from "../product/product-api-types";

/**
 * Model for the unified product search view (R7 step 2).
 *
 * Everything the view must not get wrong lives here as pure functions, because
 * the server's rules are the kind that fail silently in a UI:
 *
 * - a cursor is bound to the term *and* the scope it was issued for, so a
 *   changed term or scope must drop it rather than reuse it (the server answers
 *   a mismatch with 400 `product_invalid_input`);
 * - the workspace scope cannot answer a term shorter than three characters at
 *   all (no trigram index outside the session scope), so that combination must
 *   not be rendered as "no results";
 * - a page that came back short *with* a cursor is a bounded read, not the end
 *   of the history, and must say so;
 * - a workspace page is ordered by `(session_id, seq)`, so "which sessions
 *   match" is a client-side grouping the server deliberately does not send.
 */

export const PRODUCT_SEARCH_SCOPE_KINDS = ["session", "workspace", "trace"] as const;
export type ProductSearchScopeKind = (typeof PRODUCT_SEARCH_SCOPE_KINDS)[number];

export interface ProductSearchScope {
  kind: ProductSearchScopeKind;
  id: string;
}

/** Shortest term the workspace scope can answer; below it the API returns 400. */
export const MIN_WORKSPACE_SEARCH_TERM_CHARS = 3;

/** Longest term the API accepts, in UTF-8 bytes. */
export const MAX_SEARCH_TERM_BYTES = 128;

/** Page size the view asks for; the API's own default is 32. */
export const SEARCH_PAGE_LIMIT = 32;

export function formatSearchScope(scope: ProductSearchScope): string {
  return `${scope.kind}:${scope.id}`;
}

/**
 * Parse a canonical scope string. Strict on purpose: the response echoes the
 * scope it resolved, and the view keys its state on that echo, so an id with a
 * separator or whitespace in it is not a scope this client should build.
 */
export function parseSearchScope(value: string): ProductSearchScope | null {
  const separator = value.indexOf(":");
  if (separator <= 0) {
    return null;
  }
  const kind = value.slice(0, separator);
  const id = value.slice(separator + 1);
  if (!(PRODUCT_SEARCH_SCOPE_KINDS as readonly string[]).includes(kind)) {
    return null;
  }
  if (!id || /[\u0000-\u001f\u007f-\u009f:\s]/u.test(id)) {
    return null;
  }
  return { kind: kind as ProductSearchScopeKind, id };
}

export interface SearchScopeOption {
  kind: ProductSearchScopeKind;
  available: boolean;
  /** Why the scope cannot be used right now; null when it is available. */
  reasonCopyKey: string | null;
}

export function searchScopeOptions(input: {
  sessionId: string | null;
  workspaceId: string | null;
}): SearchScopeOption[] {
  return [
    {
      kind: "session",
      available: Boolean(input.sessionId),
      reasonCopyKey: input.sessionId ? null : "search.scopeSessionUnavailable",
    },
    {
      kind: "workspace",
      available: Boolean(input.workspaceId),
      reasonCopyKey: input.workspaceId ? null : "search.scopeWorkspaceUnavailable",
    },
    {
      kind: "trace",
      available: Boolean(input.sessionId),
      reasonCopyKey: input.sessionId ? null : "search.scopeTraceUnavailable",
    },
  ];
}

/** The workspace scope's own term rule; the API refuses shorter terms. */
export function workspaceScopeTermOk(term: string): boolean {
  return [...term].length >= MIN_WORKSPACE_SEARCH_TERM_CHARS;
}

export function searchTermBytes(term: string): number {
  return new TextEncoder().encode(term).length;
}

export interface SearchRequest {
  q: string;
  scope: string;
  cursor?: string;
  limit: number;
}

/** A first-page request: no cursor, because none exists for this pair yet. */
export function searchPageRequest(
  term: string,
  scope: ProductSearchScope,
  limit = SEARCH_PAGE_LIMIT,
): SearchRequest {
  return { q: term, scope: formatSearchScope(scope), limit };
}

/**
 * The request for the next page of an *unchanged* term and scope.
 *
 * Returns null when there is no next page, when the server echoed a different
 * scope than the one asked for (the page does not belong to this search), or
 * when the cursor is missing — a cursor is only ever used with the exact pair
 * that produced it.
 */
export function nextSearchPageRequest(
  current: { term: string; scope: ProductSearchScope; limit?: number },
  page: { scope: string; next_cursor?: string },
): SearchRequest | null {
  if (!page.next_cursor) {
    return null;
  }
  if (page.scope !== formatSearchScope(current.scope)) {
    return null;
  }
  return {
    q: current.term,
    scope: page.scope,
    cursor: page.next_cursor,
    limit: current.limit ?? SEARCH_PAGE_LIMIT,
  };
}

/**
 * Whether a page is a bounded read rather than a complete answer.
 *
 * Fewer hits than asked for *and* a cursor means the scan stopped on its own
 * budget (the trace scope shares a byte budget across the runs of one request).
 * Saying "no more matches" there would present a partial answer as complete.
 */
export function searchPartial(input: {
  hitCount: number;
  limit: number;
  nextCursor?: string;
}): boolean {
  return Boolean(input.nextCursor) && input.hitCount < input.limit;
}

export interface SearchSessionGroup {
  sessionId: string;
  hits: ProductSearchHit[];
}

/** Group a page by session, preserving the order the server sent. */
export function groupHitsBySession(
  hits: readonly ProductSearchHit[],
): SearchSessionGroup[] {
  const groups: SearchSessionGroup[] = [];
  const bySession = new Map<string, SearchSessionGroup>();
  for (const hit of hits) {
    let group = bySession.get(hit.session_id);
    if (!group) {
      group = { sessionId: hit.session_id, hits: [] };
      bySession.set(hit.session_id, group);
      groups.push(group);
    }
    group.hits.push(hit);
  }
  return groups;
}

export interface SearchHitFacts {
  /** Which corpus the hit came from, as a copy key. */
  sourceCopyKey: string;
  /** Present only when the API sent a binding ordinal. */
  runOrdinal: number | null;
  /** False when the API sent no timestamp (a legacy trace line). */
  hasCreatedAt: boolean;
  /** Only a message hit has a ledger sequence that identifies a message. */
  locatable: boolean;
}

/**
 * The facts one hit may render. Nothing here is derived from anything but the
 * response: an absent `run_ordinal` stays absent, and a missing `created_at` is
 * reported as missing rather than filled in with a client clock.
 */
export function describeSearchHit(hit: ProductSearchHit): SearchHitFacts {
  return {
    sourceCopyKey: hit.source === "trace" ? "search.hitTrace" : "search.hitMessage",
    runOrdinal: hit.run_ordinal ?? null,
    hasCreatedAt: Boolean(hit.created_at),
    locatable: hit.source === "message",
  };
}

/**
 * What to do when the API answers 409 `product_events_expired`.
 *
 * With a cursor the run it pointed at is gone (or its trace was cleaned), so
 * the cursor must be dropped and the search restarted from the first page. A
 * first page that expires has no cursor to drop — restarting would repeat the
 * same request forever — so the state is reported instead, as "the content no
 * longer exists" rather than "no results".
 */
export function expiredRecovery(input: { hadCursor: boolean }): "restart" | "report" {
  return input.hadCursor ? "restart" : "report";
}

/**
 * Append one page to the loaded hits. A page whose echoed scope differs from
 * the current search is refused instead of appended: the two answer different
 * questions, and merging them would show hits for a scope the user is not in.
 */
export function mergeSearchPage(
  current: { scope: ProductSearchScope; hits: ProductSearchHit[] },
  page: { scope: string; hits: ProductSearchHit[] },
): ProductSearchHit[] | null {
  if (page.scope !== formatSearchScope(current.scope)) {
    return null;
  }
  return [...current.hits, ...page.hits];
}
