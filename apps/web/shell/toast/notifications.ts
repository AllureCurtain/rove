/**
 * Which facts deserve a toast, and which do not.
 *
 * The design keeps inline surfaces as they are and adds toasts for work that
 * happens outside the reader's view. These two decisions are pure so the rule
 * is testable without a browser: an existing inline result is never repeated as
 * a toast, and only a *change* into failure is news.
 */

import type { ToastKind } from "./toast-store";

export interface BackgroundSessionLike {
  id: string;
  title: string;
  status: string;
}

/**
 * Sessions that just moved into `error`, given the statuses already observed.
 *
 * A session first seen in `error` is not news: it may have failed before this
 * window opened. The active session is skipped because the conversation itself
 * is showing that failure inline.
 */
export function newBackgroundFailures(
  known: ReadonlyMap<string, string>,
  sessions: ReadonlyArray<BackgroundSessionLike>,
  activeSessionId: string | null,
): { failures: BackgroundSessionLike[]; known: Map<string, string> } {
  const failures: BackgroundSessionLike[] = [];
  const next = new Map<string, string>();
  for (const session of sessions) {
    next.set(session.id, session.status);
    const previous = known.get(session.id);
    if (previous === undefined || previous === "error" || session.status !== "error") {
      continue;
    }
    if (session.id === activeSessionId) {
      continue;
    }
    failures.push(session);
  }
  return { failures, known: next };
}

export type ProviderTestOutcome = "pass" | "incomplete" | "fail";

/**
 * The provider test reports inline while its settings panel is mounted. If the
 * reader left that panel (or the settings route) while the request was in
 * flight, the inline result can no longer appear, so the toast is the only
 * surface left — and then it must appear.
 */
export function providerTestToastKind(
  mounted: boolean,
  outcome: ProviderTestOutcome,
): ToastKind | null {
  if (mounted) {
    return null;
  }
  if (outcome === "fail") {
    return "error";
  }
  return outcome === "incomplete" ? "info" : "success";
}
