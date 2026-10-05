import type { SettingsSectionId } from "../../settings/sections";

/**
 * Toast queue state (design F5).
 *
 * The provider owns timers and the DOM; this module owns the decisions that are
 * easy to get subtly wrong and worth testing without a browser:
 *
 * - how many toasts may share the screen, and which one leaves first;
 * - how long each kind stays, and what hovering does to that;
 * - which repeated notifications are suppressed (a background session that
 *   fails twice in five minutes is one piece of news, not two);
 * - which toasts the notification inbox remembers, and which of those the
 *   reader has already read.
 */

export const TOAST_MAX_VISIBLE = 3;
export const TOAST_FAILURE_HISTORY_LIMIT = 50;
export const TOAST_DEDUPE_LEDGER_LIMIT = 64;
export const TOAST_REDUCED_MOTION_QUERY = "(prefers-reduced-motion: reduce)";

export const TOAST_DURATION_MS = {
  success: 4_000,
  info: 4_000,
  error: 8_000,
} as const;

export const TOAST_DEDUPE_WINDOW_MS = 5 * 60_000;

export type ToastKind = keyof typeof TOAST_DURATION_MS;

/**
 * Where a toast points, so the inbox can offer the one action that is useful
 * from a list: going to the thing that failed. Absent means the notification is
 * informational and the row stays inert.
 *
 * The settings arm borrows the section id type so a producer cannot name a
 * destination that does not exist; the module itself stays dependency-free.
 */
export type ToastTarget =
  | { kind: "session"; sessionId: string }
  | { kind: "settings"; section: SettingsSectionId };

export interface ToastInput {
  kind: ToastKind;
  message: string;
  /**
   * Same key inside `TOAST_DEDUPE_WINDOW_MS` means the same news. The background
   * session failure uses the session id.
   */
  dedupeKey?: string;
  target?: ToastTarget;
}

export interface ToastEntry {
  id: string;
  kind: ToastKind;
  message: string;
  createdAt: number;
  /** Absolute deadline; hover shifts it, it is never recomputed from scratch. */
  expiresAt: number;
  dedupeKey?: string;
  target?: ToastTarget;
}

export interface ToastState {
  visible: ToastEntry[];
  /** Error toasts, oldest first, for the notification inbox. */
  failures: ToastEntry[];
  /**
   * The newest failure that was in the list when the inbox was last opened, or
   * null when it never was.
   *
   * This is an identity rather than a timestamp on purpose: a failure and the
   * open that follows it can share a millisecond, and no comparison of two
   * clocks can separate them. Everything after this entry in `failures` is
   * unread.
   */
  inboxReadThrough: string | null;
  /** Dedupe ledger: what was already said, and when. */
  recent: Array<{ key: string; at: number }>;
  /** When the stack was hovered, or null when the timers run. */
  pausedAt: number | null;
}

export const EMPTY_TOAST_STATE: ToastState = {
  visible: [],
  failures: [],
  inboxReadThrough: null,
  recent: [],
  pausedAt: null,
};

export type ToastEvent =
  | { type: "notify"; toast: ToastEntry }
  | { type: "dismiss"; id: string }
  | { type: "expire"; id: string; now: number }
  | { type: "pause"; now: number }
  | { type: "resume"; now: number }
  | { type: "markInboxRead" }
  | { type: "clearInbox" };

export function createToastEntry(
  input: ToastInput,
  id: string,
  now: number,
): ToastEntry {
  return {
    id,
    kind: input.kind,
    message: input.message,
    createdAt: now,
    expiresAt: now + TOAST_DURATION_MS[input.kind],
    ...(input.dedupeKey === undefined ? {} : { dedupeKey: input.dedupeKey }),
    ...(input.target === undefined ? {} : { target: input.target }),
  };
}

/**
 * The inbox, newest first: a failure list is read from the most recent event
 * backwards, unlike the toast stack which shows arrival order.
 */
export function inboxFailures(state: ToastState): ToastEntry[] {
  return [...state.failures].reverse();
}

/**
 * The ids of the failures that arrived after the list was last opened.
 *
 * A marker that is no longer in the list (it fell off the cap, or the list was
 * cleared) leaves everything present unread, which is the honest reading: those
 * entries cannot have been in the list the reader opened.
 */
export function unreadFailureIds(state: ToastState): ReadonlySet<string> {
  const unread = new Set<string>();
  if (state.inboxReadThrough === null) {
    for (const toast of state.failures) {
      unread.add(toast.id);
    }
    return unread;
  }
  const marker = state.failures.findIndex(
    (toast) => toast.id === state.inboxReadThrough,
  );
  for (const toast of state.failures.slice(marker + 1)) {
    unread.add(toast.id);
  }
  return unread;
}

/** How many failures arrived after the reader last opened the inbox. */
export function unreadFailureCount(state: ToastState): number {
  return unreadFailureIds(state).size;
}

function pruneLedger(ledger: ToastState["recent"], now: number): ToastState["recent"] {
  return ledger
    .filter((entry) => now - entry.at < TOAST_DEDUPE_WINDOW_MS)
    .slice(-TOAST_DEDUPE_LEDGER_LIMIT);
}

function isDuplicate(state: ToastState, toast: ToastEntry): boolean {
  if (toast.dedupeKey === undefined) {
    return false;
  }
  return state.recent.some(
    (entry) =>
      entry.key === toast.dedupeKey &&
      toast.createdAt - entry.at < TOAST_DEDUPE_WINDOW_MS,
  );
}

export function reduceToasts(state: ToastState, event: ToastEvent): ToastState {
  switch (event.type) {
    case "notify": {
      const { toast } = event;
      if (isDuplicate(state, toast)) {
        // Same news inside the window: keep the ledger as it is so the window
        // is measured from the notification the reader actually saw.
        return state;
      }
      const recent =
        toast.dedupeKey === undefined
          ? pruneLedger(state.recent, toast.createdAt)
          : pruneLedger(
              [...state.recent, { key: toast.dedupeKey, at: toast.createdAt }],
              toast.createdAt,
            );
      const visible = [...state.visible, toast].slice(-TOAST_MAX_VISIBLE);
      const failures =
        toast.kind === "error"
          ? [...state.failures, toast].slice(-TOAST_FAILURE_HISTORY_LIMIT)
          : state.failures;
      return {
        visible,
        failures,
        inboxReadThrough: state.inboxReadThrough,
        recent,
        pausedAt: state.pausedAt,
      };
    }
    case "dismiss":
      return {
        ...state,
        visible: state.visible.filter((toast) => toast.id !== event.id),
      };
    case "expire": {
      const target = state.visible.find((toast) => toast.id === event.id);
      if (!target || state.pausedAt !== null || event.now < target.expiresAt) {
        return state;
      }
      return {
        ...state,
        visible: state.visible.filter((toast) => toast.id !== event.id),
      };
    }
    case "pause":
      return state.pausedAt === null ? { ...state, pausedAt: event.now } : state;
    case "resume": {
      if (state.pausedAt === null) {
        return state;
      }
      const elapsed = Math.max(0, event.now - state.pausedAt);
      return {
        ...state,
        visible: state.visible.map((toast) => ({
          ...toast,
          expiresAt: toast.expiresAt + elapsed,
        })),
        pausedAt: null,
      };
    }
    /**
     * Reading the inbox is a fact about the reader, not about the toasts: the
     * entries stay, and a failure that arrives afterwards is unread again even
     * while the panel is open.
     */
    case "markInboxRead": {
      const newest = state.failures.at(-1)?.id ?? null;
      return newest === state.inboxReadThrough
        ? state
        : { ...state, inboxReadThrough: newest };
    }
    /**
     * Emptying the inbox must not pull a toast off the screen: the stack and the
     * list are separate surfaces, and the visible one leaves on its own timer.
     */
    case "clearInbox":
      return state.failures.length === 0 ? state : { ...state, failures: [] };
    default:
      return state;
  }
}

/** Milliseconds until this toast leaves, given the current clock. */
export function toastRemainingMs(toast: ToastEntry, now: number): number {
  return Math.max(0, toast.expiresAt - now);
}

/**
 * Reduced motion means the toast appears and disappears without the slide: the
 * viewport reads this once and marks itself, so the CSS does the rest.
 */
export function readToastMotion(
  matchMedia: ((query: string) => { matches: boolean }) | undefined,
): "reduced" | "full" {
  if (typeof matchMedia !== "function") {
    return "full";
  }
  return matchMedia(TOAST_REDUCED_MOTION_QUERY).matches ? "reduced" : "full";
}

/** `alert` interrupts; `status` waits for a pause. Errors are the former. */
export function toastRole(kind: ToastKind): "alert" | "status" {
  return kind === "error" ? "alert" : "status";
}
