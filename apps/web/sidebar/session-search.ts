/**
 * Sidebar session search.
 *
 * Matching is substring and case-insensitive. Subsequence matching makes a
 * miss meaningless for text without word boundaries, so a query that does not
 * occur verbatim finds nothing. An empty query restores the full list
 * immediately, without waiting for the debounce.
 */

export const SESSION_SEARCH_DEBOUNCE_MS = 180;

export function normalizeSearch(value: string): string {
  return value.trim().toLocaleLowerCase();
}

export function sessionMatchesQuery(title: string, query: string): boolean {
  const normalized = normalizeSearch(query);
  if (normalized === "") {
    return true;
  }
  return title.toLocaleLowerCase().includes(normalized);
}

export interface SearchResult<T> {
  matches: T[];
  /** True when the query found nothing, as opposed to there being no query. */
  empty: boolean;
}

export function filterSessions<T extends { title: string }>(
  sessions: readonly T[],
  query: string,
): SearchResult<T> {
  const normalized = normalizeSearch(query);
  if (normalized === "") {
    return { matches: [...sessions], empty: false };
  }
  const matches = sessions.filter((session) => sessionMatchesQuery(session.title, normalized));
  return { matches, empty: matches.length === 0 };
}

/**
 * A request id that lets a late response be dropped. Each new query bumps the
 * token; a callback only applies its result when the token still matches.
 */
export function nextSearchToken(current: number): number {
  return current + 1;
}

export function isCurrentSearch(token: number, current: number): boolean {
  return token === current;
}
