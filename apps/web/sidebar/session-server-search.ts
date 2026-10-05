import type { SessionRecord } from "../state/product-types";

import { filterSessions, normalizeSearch } from "./session-search";

/**
 * Server-side session search (design F11 / §12).
 *
 * The catalog is loaded page by page until the cursor stops
 * (`listSessionsBounded`), so a local substring is normally the same answer as
 * the server's `title LIKE` query. This is the escape hatch for the case where
 * the directory is *not* the whole truth — more sessions than the catalog's page
 * ceiling, or a listing the client never received. When the server answers, its
 * rows are authoritative; when it cannot, the local substring stands as the
 * offline degradation.
 *
 * Recorded difference: the server folds ASCII case only, while the local filter
 * uses `toLocaleLowerCase`. For a non-ASCII title the two can disagree, and the
 * server wins whenever it answered.
 */
export const SESSION_SERVER_SEARCH_LIMIT = 200;

/** The query to send, or `null` when there is nothing to ask about. */
export function serverSearchQuery(query: string): string | null {
  const trimmed = query.trim();
  return trimmed.length > 0 ? trimmed : null;
}

export interface SessionRowResolution {
  /** Rows the workspace group renders. */
  sessions: SessionRecord[];
  /** Whether the group appears at all. */
  visible: boolean;
}

/**
 * Decide which rows a workspace shows while a query is active.
 *
 * `serverMatches` is `null` whenever the server has not answered — no query, a
 * query still in flight, or a failed request — and then the local substring
 * decides, exactly as before this feature existed.
 */
export function resolveSessionRows({
  localSessions,
  serverMatches,
  query,
  workspaceMatches,
}: {
  localSessions: SessionRecord[];
  serverMatches: SessionRecord[] | null;
  query: string;
  workspaceMatches: boolean;
}): SessionRowResolution {
  if (normalizeSearch(query) === "") {
    // An empty query restores every loaded row immediately.
    return { sessions: localSessions, visible: true };
  }
  if (workspaceMatches) {
    // The workspace name matched: its rows are shown unfiltered, as before.
    return { sessions: localSessions, visible: true };
  }
  const sessions =
    serverMatches ?? filterSessions(localSessions, normalizeSearch(query)).matches;
  return { sessions, visible: sessions.length > 0 };
}
