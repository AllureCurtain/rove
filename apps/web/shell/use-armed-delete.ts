"use client";

import { useEffect, useState } from "react";

/** How long an armed destructive action stays armed before it disarms itself. */
export const ARMED_DELETE_MS = 3200;

/**
 * A destructive action that needs two clicks, ported from PI-Desktop's
 * `hooks/use-armed-delete.ts`.
 *
 * The first click arms the action and the caller relabels the control; the arm
 * expires on its own, so a surface is never one stray click away from a
 * permanent change. The armed key is opaque to the hook, so a caller can arm
 * either a row or one item of a row menu.
 *
 * Covered by `tests/e2e/sidebar-armed-delete.spec.ts` rather than a unit test:
 * the behaviour is a timer plus a click sequence, and this app has no DOM
 * harness for hooks (its hook tests cover pure exports only).
 */
export function useArmedDelete(timeoutMs: number = ARMED_DELETE_MS) {
  const [armed, setArmed] = useState<string | null>(null);

  useEffect(() => {
    if (!armed) {
      return;
    }
    const timer = setTimeout(() => setArmed(null), timeoutMs);
    return () => clearTimeout(timer);
  }, [armed, timeoutMs]);

  return { armed, setArmed };
}
