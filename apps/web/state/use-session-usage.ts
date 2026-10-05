"use client";

import { useEffect, useMemo, useState } from "react";

import { createProductApiClient } from "../product/product-client";
import type { ProductSessionUsageResponse } from "../product/product-api-types";

export type SessionUsageState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "ready"; data: ProductSessionUsageResponse }
  | { status: "error"; message: string };

/**
 * Loads `/product/sessions/{id}/usage` for the inspector. Refetches when the
 * session changes or a run stops being busy so durable report totals appear
 * after terminal. Failures stay non-blocking: the live run usage still shows.
 */
export function useSessionUsage(
  sessionId: string | null,
  busy: boolean,
): SessionUsageState {
  const [state, setState] = useState<SessionUsageState>({ status: "idle" });
  const client = useMemo(() => createProductApiClient(), []);

  useEffect(() => {
    if (!sessionId) {
      setState({ status: "idle" });
      return;
    }
    const id = sessionId;
    let cancelled = false;

    async function load() {
      setState((prev) =>
        prev.status === "ready" ? prev : { status: "loading" },
      );
      try {
        const data = await client.getSessionUsage(id);
        if (!cancelled) {
          setState({ status: "ready", data });
        }
      } catch (error) {
        if (!cancelled) {
          const message =
            error instanceof Error ? error.message : "Failed to load usage";
          setState({ status: "error", message });
        }
      }
    }

    void load();
    return () => {
      cancelled = true;
    };
  }, [sessionId, busy, client]);

  return state;
}
