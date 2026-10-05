"use client";

import { useCallback, useEffect, useState } from "react";

/**
 * Whether the left navigation rail is collapsed (design §3.1).
 *
 * Like the rail width and the work panel, this is UI-only layout state: the
 * product preference contract carries no layout values, so it persists in
 * localStorage under its own key. A reload restores the rail exactly as it was
 * left, and a missing or malformed value means the default expanded rail.
 */
const STORAGE_KEY = "rove.ui-nav-collapsed";

function readStoredCollapsed(): boolean {
  if (typeof window === "undefined") {
    return false;
  }
  try {
    return window.localStorage.getItem(STORAGE_KEY) === "1";
  } catch {
    return false;
  }
}

function writeStoredCollapsed(collapsed: boolean): void {
  if (typeof window === "undefined") {
    return;
  }
  try {
    window.localStorage.setItem(STORAGE_KEY, collapsed ? "1" : "0");
  } catch {
    // Optional preference; the in-memory state still applies.
  }
}

export function useNavCollapsed(): {
  collapsed: boolean;
  setCollapsed: (collapsed: boolean) => void;
} {
  const [collapsed, setCollapsedState] = useState(false);

  // Restore after mount so the server and the first client render agree.
  useEffect(() => {
    setCollapsedState(readStoredCollapsed());
  }, []);

  const setCollapsed = useCallback((next: boolean) => {
    setCollapsedState(next);
    writeStoredCollapsed(next);
  }, []);

  return { collapsed, setCollapsed };
}
