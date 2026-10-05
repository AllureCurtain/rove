"use client";

import { useCallback, useSyncExternalStore } from "react";

import {
  migrateOverrides,
  type ShortcutBinding,
  type ShortcutOverride,
} from "./keybindings";

const KEYBINDINGS_KEY = "rove.ui-keybindings";

export type KeybindingOverrides = Record<string, ShortcutOverride>;

/** Action ids that were renamed; old stored ids fold into the replacement. */
const RETIRED_ACTION_IDS: Record<string, string> = {};

function sanitize(value: unknown): KeybindingOverrides {
  if (typeof value !== "object" || value === null) {
    return {};
  }
  const overrides: KeybindingOverrides = {};
  for (const [id, entry] of Object.entries(value as Record<string, unknown>)) {
    if (entry === null) {
      overrides[id] = null;
      continue;
    }
    if (typeof entry !== "object") {
      continue;
    }
    const binding = entry as Partial<ShortcutBinding>;
    if (
      typeof binding.key === "string" &&
      binding.key.length > 0 &&
      typeof binding.primary === "boolean" &&
      typeof binding.shift === "boolean" &&
      typeof binding.alt === "boolean"
    ) {
      overrides[id] = {
        key: binding.key,
        primary: binding.primary,
        shift: binding.shift,
        alt: binding.alt,
      };
    }
  }
  return migrateOverrides(overrides, RETIRED_ACTION_IDS);
}

function readStored(): KeybindingOverrides {
  if (typeof window === "undefined") {
    return {};
  }
  try {
    const raw = window.localStorage.getItem(KEYBINDINGS_KEY);
    if (!raw) {
      return {};
    }
    return sanitize(JSON.parse(raw));
  } catch {
    return {};
  }
}

function writeStored(overrides: KeybindingOverrides): void {
  if (typeof window === "undefined") {
    return;
  }
  // Only differences from the default are ever written, so a future change to
  // a default reaches everyone who did not explicitly keep the old one.
  if (Object.keys(overrides).length === 0) {
    window.localStorage.removeItem(KEYBINDINGS_KEY);
    return;
  }
  window.localStorage.setItem(KEYBINDINGS_KEY, JSON.stringify(overrides));
}

let cached: KeybindingOverrides | null = null;
const listeners = new Set<() => void>();

function snapshot(): KeybindingOverrides {
  if (cached === null) {
    cached = readStored();
  }
  return cached;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function publish(next: KeybindingOverrides): void {
  cached = next;
  writeStored(next);
  listeners.forEach((listener) => listener());
}

export function setKeybindingOverride(
  actionId: string,
  override: ShortcutOverride | undefined,
): void {
  const current = snapshot();
  if (override === undefined) {
    if (!(actionId in current)) {
      return;
    }
    const { [actionId]: _removed, ...rest } = current;
    publish(rest);
    return;
  }
  publish({ ...current, [actionId]: override });
}

export function useKeybindingOverrides(): KeybindingOverrides {
  return useSyncExternalStore(
    subscribe,
    snapshot,
    // Server render has no stored overrides.
    () => ({}),
  );
}

/** Stable callback pair for the settings UI. */
export function useKeybindingOverrideActions(): {
  apply: (actionId: string, override: ShortcutOverride | undefined) => void;
  resetAll: () => void;
} {
  const apply = useCallback(
    (actionId: string, override: ShortcutOverride | undefined) => {
      setKeybindingOverride(actionId, override);
    },
    [],
  );
  const resetAll = useCallback(() => {
    publish({});
  }, []);
  return { apply, resetAll };
}
