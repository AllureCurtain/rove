"use client";

import { useCallback, useEffect, useState } from "react";

import {
  READING_WIDTH_DEFAULT,
  clampReadingWidth,
  parseStoredReadingWidth,
  resolveReadingWidth,
} from "./reading-width";

/**
 * The conversation reading width as a UI preference.
 *
 * The product preference contract is a typed server-side field and does not
 * carry layout values (design §2.2), so this lives in localStorage with the
 * other layout preferences (`rove.ui-sidebar-width`, `rove.ui-work-panel-width`).
 * PI-Desktop stores the same number in app settings; the storage location is the
 * only difference, and the value is the same *preference* rather than a measured
 * width, so a squeezed column never rewrites it.
 */
const STORAGE_KEY = "rove.ui-reading-width";

function readStoredWidth(): number {
  if (typeof window === "undefined") {
    return READING_WIDTH_DEFAULT;
  }
  try {
    return (
      parseStoredReadingWidth(window.localStorage.getItem(STORAGE_KEY)) ??
      READING_WIDTH_DEFAULT
    );
  } catch {
    return READING_WIDTH_DEFAULT;
  }
}

function writeStoredWidth(width: number): void {
  if (typeof window === "undefined") {
    return;
  }
  try {
    window.localStorage.setItem(STORAGE_KEY, String(width));
  } catch {
    // A blocked storage quota must not break resizing.
  }
}

export function useReadingWidth(): {
  preferred: number;
  commit: (width: number, available: number) => void;
  reset: (available: number) => void;
} {
  const [preferred, setPreferred] = useState<number>(READING_WIDTH_DEFAULT);

  // Restore after mount so the server and the first client render agree.
  useEffect(() => {
    setPreferred(readStoredWidth());
  }, []);

  // Only explicit user actions persist: a clamped value is a fact about the
  // window, not a new preference.
  const commit = useCallback((width: number, available: number) => {
    const next = clampReadingWidth(resolveReadingWidth(width), available);
    setPreferred(next);
    writeStoredWidth(next);
  }, []);

  const reset = useCallback(
    (available: number) => {
      const next = clampReadingWidth(READING_WIDTH_DEFAULT, available);
      setPreferred(next);
      writeStoredWidth(next);
    },
    [],
  );

  return { preferred, commit, reset };
}
