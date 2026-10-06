"use client";

import { useEffect, useMemo, useSyncExternalStore } from "react";

import { createProductApiClient } from "../product/product-client";

/**
 * Lazy per-session message count for the sidebar hover card (design §7.3).
 *
 * The message ledger is paginated, so "how many messages" is a paged read, not
 * a field. This store is the only place that walks it:
 *
 * - one in-flight walk per session; the result — including a failure and a
 *   "more than we counted" marker — is cached, so re-hovering never re-asks;
 * - the walk is bounded by `SESSION_MESSAGE_COUNT_MAX_PAGES`; a ledger longer
 *   than that reports `count` plus `more: true` rather than lying with an
 *   exact-looking number;
 * - the cache is bounded, so browsing a long sidebar cannot grow it forever.
 */

// Must stay within the API's message page cap (128); a larger ask is a 400.
export const SESSION_MESSAGE_COUNT_PAGE_SIZE = 128;
export const SESSION_MESSAGE_COUNT_MAX_PAGES = 5;
export const SESSION_MESSAGE_COUNT_LIMIT = 64;

export type SessionMessageCount =
  | { status: "loading" }
  | { status: "ready"; count: number; more: boolean }
  | { status: "unavailable" };

export interface SessionMessageCountStore {
  read(sessionId: string): SessionMessageCount | null;
  request(sessionId: string): void;
  subscribe(listener: () => void): () => void;
  /** Test seam: drop every cached entry. */
  clear(): void;
}

export type SessionMessageCountLoader = (
  sessionId: string,
) => Promise<SessionMessageCount>;

const UNAVAILABLE: SessionMessageCount = { status: "unavailable" };

export function createSessionMessageCountStore(
  loader: SessionMessageCountLoader,
  limit = SESSION_MESSAGE_COUNT_LIMIT,
): SessionMessageCountStore {
  const entries = new Map<string, SessionMessageCount>();
  const listeners = new Set<() => void>();

  function publish() {
    for (const listener of listeners) {
      listener();
    }
  }

  function remember(sessionId: string, count: SessionMessageCount) {
    // Re-inserting moves the entry to the end, so eviction drops the row that
    // was looked at longest ago — never the row being written right now.
    entries.delete(sessionId);
    entries.set(sessionId, count);
    while (entries.size > limit) {
      const oldest = [...entries.keys()].find((key) => key !== sessionId);
      if (oldest === undefined) {
        break;
      }
      entries.delete(oldest);
    }
    publish();
  }

  return {
    read(sessionId) {
      return entries.get(sessionId) ?? null;
    },
    request(sessionId) {
      if (entries.get(sessionId)) {
        return;
      }
      remember(sessionId, { status: "loading" });
      void loader(sessionId).then(
        (count) => remember(sessionId, count),
        () => remember(sessionId, UNAVAILABLE),
      );
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    clear() {
      entries.clear();
      publish();
    },
  };
}

async function defaultLoader(sessionId: string): Promise<SessionMessageCount> {
  const client = createProductApiClient();
  let count = 0;
  let afterSeq: number | undefined;
  for (let page = 0; page < SESSION_MESSAGE_COUNT_MAX_PAGES; page += 1) {
    const response = await client.listMessages(sessionId, {
      afterSeq,
      limit: SESSION_MESSAGE_COUNT_PAGE_SIZE,
    });
    count += response.messages.length;
    if (response.next_after_seq === undefined) {
      return { status: "ready", count, more: false };
    }
    afterSeq = response.next_after_seq;
  }
  // The ledger ran past the page budget: report what was counted and mark it
  // partial instead of presenting a truncated number as exact.
  return { status: "ready", count, more: true };
}

export const sessionMessageCounts =
  createSessionMessageCountStore(defaultLoader);

/** Read (and lazily request) the message count for one session. */
export function useSessionMessageCount(
  sessionId: string | null,
): SessionMessageCount | null {
  const subscribe = useMemo(
    () => (listener: () => void) => sessionMessageCounts.subscribe(listener),
    [],
  );
  const snapshot = useMemo(
    () => () =>
      sessionId === null ? null : sessionMessageCounts.read(sessionId),
    [sessionId],
  );
  const count = useSyncExternalStore(subscribe, snapshot, () => null);

  useEffect(() => {
    if (sessionId !== null) {
      sessionMessageCounts.request(sessionId);
    }
  }, [sessionId]);

  return count;
}
