"use client";

import { useEffect, useMemo, useRef, useState } from "react";

import type { SessionRecord } from "../state/product-types";
import {
  SESSION_SEARCH_DEBOUNCE_MS,
  isCurrentSearch,
  nextSearchToken,
} from "./session-search";
import { serverSearchQuery } from "./session-server-search";

export interface ServerSessionSearch {
  /**
   * The server's rows per workspace, or `null` while there is nothing
   * authoritative to show (no query, still searching, or the request failed).
   */
  matchesByWorkspace: Record<string, SessionRecord[]> | null;
  /** True once a search request has failed; the local filter is in charge. */
  unavailable: boolean;
}

/**
 * F11: ask the server for matching sessions while a query is active.
 *
 * One request per workspace, in parallel, after the same 180ms debounce the
 * local filter documents. A late answer is dropped with the existing token
 * pattern (`nextSearchToken` / `isCurrentSearch`), so typing quickly cannot let
 * an older response replace a newer one. A failure clears the server rows and
 * flips `unavailable`, which leaves the sidebar's local substring in charge.
 */
export function useServerSessionSearch({
  query,
  workspaceIds,
  search,
  enabled,
}: {
  query: string;
  workspaceIds: readonly string[];
  search?: (workspaceId: string, query: string) => Promise<SessionRecord[]>;
  enabled: boolean;
}): ServerSessionSearch {
  const [matchesByWorkspace, setMatchesByWorkspace] = useState<
    Record<string, SessionRecord[]> | null
  >(null);
  const [unavailable, setUnavailable] = useState(false);
  const tokenRef = useRef(0);
  // A joined key keeps the effect dependent on the ids themselves rather than
  // on the array identity, which changes on every parent render.
  const workspaceKey = useMemo(() => workspaceIds.join("\u0000"), [workspaceIds]);

  useEffect(() => {
    const normalized = enabled ? serverSearchQuery(query) : null;
    if (!normalized || !search) {
      tokenRef.current = nextSearchToken(tokenRef.current);
      setMatchesByWorkspace(null);
      setUnavailable(false);
      return;
    }
    const token = nextSearchToken(tokenRef.current);
    tokenRef.current = token;
    const ids = workspaceKey ? workspaceKey.split("\u0000") : [];
    const timer = window.setTimeout(() => {
      void Promise.all(
        ids.map(async (workspaceId) => [
          workspaceId,
          await search(workspaceId, normalized),
        ] as const),
      ).then(
        (entries) => {
          if (!isCurrentSearch(token, tokenRef.current)) {
            return;
          }
          setMatchesByWorkspace(Object.fromEntries(entries));
          setUnavailable(false);
        },
        () => {
          if (!isCurrentSearch(token, tokenRef.current)) {
            return;
          }
          setMatchesByWorkspace(null);
          setUnavailable(true);
        },
      );
    }, SESSION_SEARCH_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [enabled, query, search, workspaceKey]);

  return { matchesByWorkspace, unavailable };
}
