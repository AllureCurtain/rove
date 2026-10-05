"use client";

import { BellIcon } from "@radix-ui/react-icons";
import { useCallback, useEffect, useRef, useState } from "react";

import { useCopy } from "../../copy/CopyProvider";
import { formatUtcTimestamp } from "../../lib/format-timestamp";
import { useToast } from "./ToastProvider";
import {
  TOAST_FAILURE_HISTORY_LIMIT,
  type ToastEntry,
  type ToastTarget,
} from "./toast-store";

/**
 * The notification inbox (design F5 §6.2/§6.5).
 *
 * The toast stack answers "something just happened"; this answers "what failed
 * while I was not looking". It lists the same error toasts the store already
 * keeps, newest first, and offers the one action a failure list is for: going to
 * the thing that failed.
 *
 * Three honest limits are part of the surface, not hidden behind it:
 *
 * - it holds this page's failures only — the store is memory-only, so a reload
 *   starts it empty, and the footer says so;
 * - it is bounded, and the footer says to what;
 * - a row whose toast carried no target is not a button, because there is
 *   nowhere to go.
 *
 * Opening the list is what marks it read; a failure that arrives while the list
 * is open therefore stays unread until the reader opens it again.
 */
export function NotificationInbox({
  onOpenTarget,
}: {
  /**
   * Navigate to where the failure happened; the shell owns routing. Returns
   * false when the destination is gone, which the list reports in place instead
   * of closing on a click that did nothing.
   */
  onOpenTarget: (target: ToastTarget) => boolean;
}) {
  const { t } = useCopy();
  const { inbox, unreadCount, unreadIds, markInboxRead, clearInbox } = useToast();
  const [open, setOpen] = useState(false);
  const [unresolved, setUnresolved] = useState(false);
  const bellRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);

  const close = useCallback(() => setOpen(false), []);

  // Opening is the moment the list has been seen, so it is also the moment the
  // badge resets. Closing never marks anything: nothing new was read.
  const openInbox = useCallback(() => {
    setUnresolved(false);
    markInboxRead();
    setOpen(true);
  }, [markInboxRead]);

  const toggle = useCallback(() => {
    if (open) {
      close();
      return;
    }
    openInbox();
  }, [open, close, openInbox]);

  useEffect(() => {
    if (!open) {
      return undefined;
    }
    const frame = window.requestAnimationFrame(() => panelRef.current?.focus());
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        close();
        bellRef.current?.focus();
      }
    }
    // A non-modal popover: a click anywhere outside it dismisses it, while the
    // rest of the shell stays usable behind.
    function onPointerDown(event: PointerEvent) {
      const target = event.target as Node | null;
      if (target === null) {
        return;
      }
      if (panelRef.current?.contains(target) || bellRef.current?.contains(target)) {
        return;
      }
      close();
    }
    window.addEventListener("keydown", onKeyDown);
    document.addEventListener("pointerdown", onPointerDown);
    return () => {
      window.cancelAnimationFrame(frame);
      window.removeEventListener("keydown", onKeyDown);
      document.removeEventListener("pointerdown", onPointerDown);
    };
  }, [open, close]);

  function openEntry(entry: ToastEntry) {
    if (entry.target === undefined) {
      return;
    }
    if (!onOpenTarget(entry.target)) {
      // The row named something the shell no longer has. The reader is looking
      // at this list, so the answer stays here rather than in a surface the
      // other view may not even render.
      setUnresolved(true);
      return;
    }
    setUnresolved(false);
    close();
    // The row that was just activated is gone; focus goes back to the trigger,
    // which is where the reader was before opening the list.
    bellRef.current?.focus();
  }

  function clear() {
    clearInbox();
    // Clearing disables the button that was just activated, which would drop
    // focus onto the document body; the panel stays open, so focus stays with it.
    panelRef.current?.focus();
  }

  function actionLabel(entry: ToastEntry): string | null {
    if (entry.target === undefined) {
      return null;
    }
    return entry.target.kind === "session" ? t("inbox.openSession") : t("inbox.openSettings");
  }

  return (
    <div className="notification-inbox">
      <button
        ref={bellRef}
        type="button"
        className="ghost icon-button notification-inbox__bell"
        onClick={toggle}
        aria-expanded={open}
        // Only while the panel exists: the attribute names an element, and a
        // reference to nothing helps no one.
        aria-controls={open ? "notification-inbox-panel" : undefined}
        data-unread={unreadCount > 0 ? "true" : undefined}
        aria-label={
          unreadCount > 0
            ? t("inbox.openUnread", { count: unreadCount })
            : t("inbox.open")
        }
      >
        <BellIcon />
        {unreadCount > 0 ? (
          <span className="notification-inbox__badge" aria-hidden="true">
            {unreadCount}
          </span>
        ) : null}
      </button>
      {open ? (
        <div
          ref={panelRef}
          id="notification-inbox-panel"
          className="notification-inbox__panel"
          role="region"
          aria-label={t("inbox.title")}
          tabIndex={-1}
        >
          <div className="notification-inbox__head">
            <div>
              <h2>{t("inbox.title")}</h2>
              <p className="notification-inbox__lede">{t("inbox.lede")}</p>
            </div>
            <button
              type="button"
              className="ghost"
              onClick={clear}
              disabled={inbox.length === 0}
            >
              {t("inbox.clear")}
            </button>
          </div>
          {unresolved ? (
            <p className="notification-inbox__unresolved" role="alert">
              {t("inbox.targetMissing")}
            </p>
          ) : null}
          {inbox.length === 0 ? (
            <p className="notification-inbox__empty" role="status">
              {t("inbox.empty")}
            </p>
          ) : (
            <ul className="notification-inbox__list">
              {inbox.map((entry) => {
                const action = actionLabel(entry);
                const created = new Date(entry.createdAt).toISOString();
                const body = (
                  <>
                    <span className="notification-inbox__message">{entry.message}</span>
                    <span className="notification-inbox__meta">
                      <time dateTime={created}>
                        {formatUtcTimestamp(created)}
                      </time>
                      {action === null ? null : (
                        <span className="notification-inbox__action">{action}</span>
                      )}
                    </span>
                  </>
                );
                return (
                  <li
                    key={entry.id}
                    className="notification-inbox__item"
                    data-unread={unreadIds.has(entry.id) ? "true" : undefined}
                  >
                    {action === null ? (
                      <div className="notification-inbox__row">{body}</div>
                    ) : (
                      <button
                        type="button"
                        className="notification-inbox__row"
                        onClick={() => openEntry(entry)}
                      >
                        {body}
                      </button>
                    )}
                  </li>
                );
              })}
            </ul>
          )}
          <p className="notification-inbox__bound">
            {t("inbox.bounded", { limit: TOAST_FAILURE_HISTORY_LIMIT })}
          </p>
        </div>
      ) : null}
    </div>
  );
}
