"use client";

import { useCallback, useEffect, useState } from "react";

/**
 * Left navigation width as a UI preference (design §3.1).
 *
 * The product preference contract is a typed, server-side field and does not
 * carry layout values (design §2.2), so this lives in localStorage like the
 * existing UI-skin and work-panel width preferences. It stores no business
 * state and no permissions, and a missing or malformed value falls back to the
 * design default instead of failing.
 */

export const SIDEBAR_MIN_WIDTH = 200;
export const SIDEBAR_MAX_WIDTH = 360;
export const SIDEBAR_DEFAULT_WIDTH = 240;
/** Keyboard step per design §3.3 (open-vetta WorkPanel 397-407 equivalent). */
export const SIDEBAR_KEYBOARD_STEP = 16;
export const SIDEBAR_KEYBOARD_LARGE_STEP = 32;

const STORAGE_KEY = "rove.ui-sidebar-width";

export function boundSidebarWidth(width: number): number {
  if (!Number.isFinite(width)) {
    return SIDEBAR_DEFAULT_WIDTH;
  }
  return Math.min(
    SIDEBAR_MAX_WIDTH,
    Math.max(SIDEBAR_MIN_WIDTH, Math.round(width)),
  );
}

function readStoredWidth(): number {
  if (typeof window === "undefined") {
    return SIDEBAR_DEFAULT_WIDTH;
  }
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (raw === null) {
      return SIDEBAR_DEFAULT_WIDTH;
    }
    const parsed = Number.parseInt(raw, 10);
    return Number.isNaN(parsed) ? SIDEBAR_DEFAULT_WIDTH : boundSidebarWidth(parsed);
  } catch {
    return SIDEBAR_DEFAULT_WIDTH;
  }
}

function writeStoredWidth(width: number): void {
  if (typeof window === "undefined") {
    return;
  }
  try {
    window.localStorage.setItem(STORAGE_KEY, String(width));
  } catch {
    // A blocked or full storage quota must not break resizing.
  }
}

/**
 * Keyboard stepping for the width handle: arrows move by the design step,
 * Shift multiplies it, Home/End jump to the bounds. Returns null when the key
 * is not a resize key, so the caller can leave it alone.
 */
export function sidebarWidthKeyboardStep(
  event: { key: string; shiftKey: boolean },
  current: number,
): number | null {
  switch (event.key) {
    case "ArrowLeft":
    case "ArrowDown":
      return boundSidebarWidth(
        current - (event.shiftKey ? SIDEBAR_KEYBOARD_LARGE_STEP : SIDEBAR_KEYBOARD_STEP),
      );
    case "ArrowRight":
    case "ArrowUp":
      return boundSidebarWidth(
        current + (event.shiftKey ? SIDEBAR_KEYBOARD_LARGE_STEP : SIDEBAR_KEYBOARD_STEP),
      );
    case "Home":
      return SIDEBAR_MIN_WIDTH;
    case "End":
      return SIDEBAR_MAX_WIDTH;
    default:
      return null;
  }
}

export function useSidebarWidth(): {
  width: number;
  setWidth: (width: number) => void;
  settleWidth: (clientX: number, element: HTMLElement) => void;
  onHandleKeyDown: (event: {
    key: string;
    shiftKey: boolean;
    preventDefault: () => void;
  }) => void;
} {
  const [width, setWidthState] = useState<number>(SIDEBAR_DEFAULT_WIDTH);

  // Restore after mount so the server and the first client render agree.
  useEffect(() => {
    setWidthState(readStoredWidth());
  }, []);

  const setWidth = useCallback((next: number) => {
    const bounded = boundSidebarWidth(next);
    setWidthState(bounded);
    writeStoredWidth(bounded);
  }, []);

  // The width is committed when the drag settles. Writing on every pointermove
  // would re-render the whole rail and fight the drag.
  const settleWidth = useCallback(
    (clientX: number, element: HTMLElement) => {
      const rect = element.getBoundingClientRect();
      setWidth(clientX - rect.left);
    },
    [setWidth],
  );

  const onHandleKeyDown = useCallback(
    (event: { key: string; shiftKey: boolean; preventDefault: () => void }) => {
      const next = sidebarWidthKeyboardStep(event, width);
      if (next === null || next === width) {
        return;
      }
      event.preventDefault();
      setWidth(next);
    },
    [width, setWidth],
  );

  return { width, setWidth, settleWidth, onHandleKeyDown };
}
