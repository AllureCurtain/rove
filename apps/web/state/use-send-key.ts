"use client";

import { useSyncExternalStore } from "react";

/**
 * The composer's send-key preference (design §6.2).
 *
 * `"enter"` is the default: Enter sends and Shift+Enter adds a newline, the
 * mainstream chat convention. `"mod-enter"` restores the earlier binding for
 * readers who prefer it: Enter adds a newline and Ctrl/Cmd+Enter sends.
 * Alt+Enter is not part of the choice — it always means "interject into the
 * running turn" (and a plain send when no turn is running).
 */
export type SendKeyPreference = "enter" | "mod-enter";

const SEND_KEY_STORAGE = "rove.ui-send-key";

export const SEND_KEY_DEFAULT: SendKeyPreference = "enter";

let cached: SendKeyPreference | null = null;
const listeners = new Set<() => void>();

function snapshot(): SendKeyPreference {
  if (cached === null) {
    const raw =
      typeof window === "undefined"
        ? null
        : window.localStorage.getItem(SEND_KEY_STORAGE);
    cached = raw === "mod-enter" ? "mod-enter" : SEND_KEY_DEFAULT;
  }
  return cached;
}

export function setSendKeyPreference(next: SendKeyPreference): void {
  if (snapshot() === next) {
    return;
  }
  cached = next;
  if (typeof window !== "undefined") {
    window.localStorage.setItem(SEND_KEY_STORAGE, next);
  }
  listeners.forEach((listener) => listener());
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/**
 * One shared preference: the composer reads it for its keymap and hint, the
 * keyboard settings panel writes it, and both stay in step through the same
 * external store.
 */
export function useSendKeyPreference(): SendKeyPreference {
  return useSyncExternalStore(subscribe, snapshot, () => SEND_KEY_DEFAULT);
}
