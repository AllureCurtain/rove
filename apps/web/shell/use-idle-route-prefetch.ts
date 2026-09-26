"use client";

import { useEffect } from "react";
import { useRouter } from "next/navigation";

import {
  ROUTE_PREFETCH_FALLBACK_DELAY_MS,
  ROUTE_PREFETCH_IDLE_TIMEOUT_MS,
  shouldPrefetchRoutes,
} from "./route-prefetch";

/**
 * Warms the route payloads the user is most likely to click next, off the boot
 * path. See `route-prefetch` for the policy and its reference.
 *
 * Nothing here runs during the mount commit: the work happens in an idle
 * callback (or the timer fallback), and a shell that unmounts first cancels it.
 * Failures are swallowed on purpose — a real navigation retries and reports
 * through its own error path, and a prefetch is never allowed to surface as an
 * error the user has to read.
 */
export function useIdleRoutePrefetch(targets: readonly string[]): void {
  const router = useRouter();
  // A stable key keeps the effect from re-running for an equivalent list.
  const key = targets.join("|");

  useEffect(() => {
    if (!shouldPrefetchRoutes(process.env.NODE_ENV)) {
      return;
    }
    const wanted = key.split("|").filter(Boolean);
    if (wanted.length === 0) {
      return;
    }
    const run = () => {
      for (const href of wanted) {
        try {
          router.prefetch(href);
        } catch {
          // A prefetch that fails is not a user-visible failure.
        }
      }
    };
    if (typeof window.requestIdleCallback === "function") {
      const id = window.requestIdleCallback(run, {
        timeout: ROUTE_PREFETCH_IDLE_TIMEOUT_MS,
      });
      return () => window.cancelIdleCallback(id);
    }
    const id = window.setTimeout(run, ROUTE_PREFETCH_FALLBACK_DELAY_MS);
    return () => window.clearTimeout(id);
  }, [key, router]);
}
