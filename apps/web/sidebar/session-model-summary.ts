"use client";

import { useEffect, useMemo, useSyncExternalStore } from "react";

import { createProductApiClient } from "../product/product-client";
import { fromProductSessionModelConfig } from "../state/product-types";
import type { ProductReasoningPreference } from "../product/product-api-types";

/**
 * Lazy per-session model summary for the sidebar hover card (design F4).
 *
 * The card must not turn a hover into a request storm, so this store is the
 * only place that reads `/product/sessions/{id}/model-config`:
 *
 * - one in-flight request per session, and the result (including a failure) is
 *   cached, so re-hovering the same row never asks again;
 * - a failure is remembered as `unavailable` instead of retried, because a
 *   hover is not a reason to keep hammering a broken endpoint;
 * - the cache is bounded, so browsing a long sidebar cannot grow it forever.
 *
 * The hook reads a per-session slice, which is what makes "latest wins" fall
 * out: a slower response for a row the reader already left can only ever update
 * that row's entry.
 */

export const SESSION_MODEL_SUMMARY_LIMIT = 64;

export type SessionModelSummary =
  | { status: "loading" }
  | {
      status: "ready";
      model: string;
      reasoning: ProductReasoningPreference;
      maxSteps: number;
    }
  | { status: "unavailable" };

export interface SessionModelSummaryStore {
  read(sessionId: string): SessionModelSummary | null;
  request(sessionId: string): void;
  subscribe(listener: () => void): () => void;
  /** Test seam: drop every cached entry. */
  clear(): void;
}

export type SessionModelSummaryLoader = (sessionId: string) => Promise<SessionModelSummary>;

const UNAVAILABLE: SessionModelSummary = { status: "unavailable" };

export function createSessionModelSummaryStore(
  loader: SessionModelSummaryLoader,
  limit = SESSION_MODEL_SUMMARY_LIMIT,
): SessionModelSummaryStore {
  const entries = new Map<string, SessionModelSummary>();
  const listeners = new Set<() => void>();

  function publish() {
    for (const listener of listeners) {
      listener();
    }
  }

  function remember(sessionId: string, summary: SessionModelSummary) {
    // Re-inserting moves the entry to the end, so eviction drops the row that
    // was looked at longest ago — never the row being written right now.
    entries.delete(sessionId);
    entries.set(sessionId, summary);
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
      const existing = entries.get(sessionId);
      if (existing) {
        return;
      }
      remember(sessionId, { status: "loading" });
      void loader(sessionId).then(
        (summary) => {
          // A late answer still lands on its own row; nothing else is touched.
          remember(sessionId, summary);
        },
        () => {
          remember(sessionId, UNAVAILABLE);
        },
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

function defaultLoader(sessionId: string): Promise<SessionModelSummary> {
  return createProductApiClient()
    .getSessionModelConfig(sessionId)
    .then((config) => {
      const summary = fromProductSessionModelConfig(config);
      return {
        status: "ready",
        model: summary.model,
        reasoning: summary.reasoning,
        maxSteps: summary.maxSteps,
      } as const;
    });
}

export const sessionModelSummaries = createSessionModelSummaryStore(defaultLoader);

/** Read (and lazily request) the model summary for one session. */
export function useSessionModelSummary(sessionId: string | null): SessionModelSummary | null {
  const subscribe = useMemo(
    () => (listener: () => void) => sessionModelSummaries.subscribe(listener),
    [],
  );
  const snapshot = useMemo(
    () => () => (sessionId === null ? null : sessionModelSummaries.read(sessionId)),
    [sessionId],
  );
  const summary = useSyncExternalStore(subscribe, snapshot, () => null);

  useEffect(() => {
    if (sessionId !== null) {
      sessionModelSummaries.request(sessionId);
    }
  }, [sessionId]);

  return summary;
}
