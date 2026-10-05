"use client";

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";

import { useCopy } from "../../copy/CopyProvider";
import { resolveShellPortalHost } from "../portal-host";
import {
  EMPTY_TOAST_STATE,
  TOAST_REDUCED_MOTION_QUERY,
  createToastEntry,
  inboxFailures,
  readToastMotion,
  reduceToasts,
  toastRemainingMs,
  toastRole,
  unreadFailureIds,
  type ToastEntry,
  type ToastInput,
} from "./toast-store";

export interface ToastController {
  /** Toasts currently on screen, oldest first. */
  toasts: ToastEntry[];
  /**
   * The notification inbox: this page's error toasts, newest first, bounded by
   * the store. It is memory-only, so a reload starts it empty.
   */
  inbox: ToastEntry[];
  /** Failures that arrived after the inbox was last opened. */
  unreadCount: number;
  /** Which of `inbox` those are, so the badge and the rows agree by identity. */
  unreadIds: ReadonlySet<string>;
  notify: (input: ToastInput) => void;
  dismiss: (id: string) => void;
  pause: () => void;
  resume: () => void;
  /** Called when the inbox opens: everything in it becomes read. */
  markInboxRead: () => void;
  /** Empties the list; a toast still on screen leaves on its own timer. */
  clearInbox: () => void;
}

const ToastContext = createContext<ToastController | null>(null);

export function ToastProvider({ children }: { children: ReactNode }) {
  const { t } = useCopy();
  const [state, dispatch] = useReducer(reduceToasts, EMPTY_TOAST_STATE);
  const [motion, setMotion] = useState<"reduced" | "full">("full");
  const [host, setHost] = useState<HTMLElement | null>(null);
  const sequence = useRef(0);
  const { visible, pausedAt } = state;

  useEffect(() => {
    const media = window.matchMedia(TOAST_REDUCED_MOTION_QUERY);
    setMotion(readToastMotion((query) => window.matchMedia(query)));
    const onChange = () => setMotion(media.matches ? "reduced" : "full");
    media.addEventListener("change", onChange);
    return () => media.removeEventListener("change", onChange);
  }, []);

  // Resolved when the first toast arrives rather than on mount: the frame this
  // portal needs is rendered by a child, so it does not exist yet at mount.
  const hasToasts = visible.length > 0;
  useEffect(() => {
    setHost(
      hasToasts && typeof document !== "undefined"
        ? resolveShellPortalHost(document)
        : null,
    );
  }, [hasToasts]);

  // One timer per visible toast, re-armed whenever the stack changes. Hovering
  // pauses by shifting the deadlines, and no timer runs while paused.
  useEffect(() => {
    if (pausedAt !== null) {
      return undefined;
    }
    const now = Date.now();
    const timers = visible.map((toast) =>
      window.setTimeout(
        () => dispatch({ type: "expire", id: toast.id, now: Date.now() }),
        toastRemainingMs(toast, now),
      ),
    );
    return () => {
      for (const timer of timers) {
        window.clearTimeout(timer);
      }
    };
  }, [visible, pausedAt]);

  const notify = useCallback((input: ToastInput) => {
    sequence.current += 1;
    dispatch({
      type: "notify",
      toast: createToastEntry(input, `toast-${sequence.current}`, Date.now()),
    });
  }, []);
  const dismiss = useCallback((id: string) => dispatch({ type: "dismiss", id }), []);
  const pause = useCallback(() => dispatch({ type: "pause", now: Date.now() }), []);
  const resume = useCallback(() => dispatch({ type: "resume", now: Date.now() }), []);
  const markInboxRead = useCallback(() => dispatch({ type: "markInboxRead" }), []);
  const clearInbox = useCallback(() => dispatch({ type: "clearInbox" }), []);

  // Derived from the reducer state so the badge and the rows can never disagree
  // about what is unread.
  const inbox = useMemo(() => inboxFailures(state), [state]);
  const unreadIds = useMemo(() => unreadFailureIds(state), [state]);
  const unreadCount = unreadIds.size;

  const value = useMemo<ToastController>(
    () => ({
      toasts: visible,
      inbox,
      unreadCount,
      unreadIds,
      notify,
      dismiss,
      pause,
      resume,
      markInboxRead,
      clearInbox,
    }),
    [
      visible,
      inbox,
      unreadCount,
      unreadIds,
      notify,
      dismiss,
      pause,
      resume,
      markInboxRead,
      clearInbox,
    ],
  );

  return (
    <ToastContext.Provider value={value}>
      {children}
      {hasToasts && host
        ? createPortal(
            <div
              className="toast-viewport"
              data-motion={motion}
              role="region"
              aria-label={t("toast.regionLabel")}
              onMouseEnter={pause}
              onMouseLeave={resume}
            >
              {visible.map((toast) => (
                <div
                  key={toast.id}
                  className={`toast toast--${toast.kind}`}
                  data-kind={toast.kind}
                  role={toastRole(toast.kind)}
                >
                  <p className="toast__message">{toast.message}</p>
                  <button
                    type="button"
                    className="toast__dismiss"
                    onClick={() => dismiss(toast.id)}
                    aria-label={t("toast.dismiss")}
                  >
                    <span aria-hidden="true">×</span>
                  </button>
                </div>
              ))}
            </div>,
            host,
          )
        : null}
    </ToastContext.Provider>
  );
}

/**
 * Notifications outside the shell (a `/dev` preview, a focused unit test) have
 * no toast surface; dropping them is better than failing the component that
 * reported them. Mirrors `useCopy`'s fallback.
 */
const FALLBACK_TOASTS: ToastController = {
  toasts: [],
  inbox: [],
  unreadCount: 0,
  unreadIds: new Set<string>(),
  notify: () => undefined,
  dismiss: () => undefined,
  pause: () => undefined,
  resume: () => undefined,
  markInboxRead: () => undefined,
  clearInbox: () => undefined,
};

export function useToast(): ToastController {
  return useContext(ToastContext) ?? FALLBACK_TOASTS;
}
