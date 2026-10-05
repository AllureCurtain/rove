"use client";

import { useCallback, useSyncExternalStore } from "react";

import {
  clampFontScale,
  FONT_SCALE_DEFAULT,
  stepFontScale,
} from "./font-scale";

const FONT_SCALE_KEY = "rove.ui-font-scale";

let cached: number | null = null;
const listeners = new Set<() => void>();

function snapshot(): number {
  if (cached === null) {
    if (typeof window === "undefined") {
      cached = FONT_SCALE_DEFAULT;
    } else {
      const raw = Number(window.localStorage.getItem(FONT_SCALE_KEY));
      cached = Number.isFinite(raw) && raw !== 0 ? clampFontScale(raw) : FONT_SCALE_DEFAULT;
    }
  }
  return cached;
}

function publish(next: number): void {
  cached = next;
  if (typeof window !== "undefined") {
    window.localStorage.setItem(FONT_SCALE_KEY, String(next));
  }
  listeners.forEach((listener) => listener());
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/**
 * Reader font scale as one shared preference: the frame reads it for the CSS
 * variable, the settings control writes it, and both stay in step through the
 * same external store.
 */
export function useFontScalePreference(): {
  fontScale: number;
  stepFont: (direction: "up" | "down") => void;
  resetFont: () => void;
} {
  const fontScale = useSyncExternalStore(
    subscribe,
    snapshot,
    () => FONT_SCALE_DEFAULT,
  );
  const stepFont = useCallback((direction: "up" | "down") => {
    publish(stepFontScale(snapshot(), direction));
  }, []);
  const resetFont = useCallback(() => {
    publish(FONT_SCALE_DEFAULT);
  }, []);
  return { fontScale, stepFont, resetFont };
}
