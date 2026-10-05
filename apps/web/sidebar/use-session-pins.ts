"use client";

import { useCallback, useEffect, useMemo, useState } from "react";

import { nextPinTimestamp, prunePins, type SessionPin } from "./session-order";

const PINNED_SESSIONS_KEY = "rove.ui-pinned-sessions";

function readStoredPins(): SessionPin[] {
  if (typeof window === "undefined") {
    return [];
  }
  try {
    const raw = window.localStorage.getItem(PINNED_SESSIONS_KEY);
    if (!raw) {
      return [];
    }
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      return [];
    }
    return parsed.flatMap((entry) => {
      if (
        typeof entry !== "object" ||
        entry === null ||
        typeof (entry as SessionPin).sessionId !== "string" ||
        typeof (entry as SessionPin).pinnedAt !== "number"
      ) {
        return [];
      }
      return [entry as SessionPin];
    });
  } catch {
    return [];
  }
}

function writeStoredPins(pins: readonly SessionPin[]): void {
  if (typeof window === "undefined") {
    return;
  }
  window.localStorage.setItem(PINNED_SESSIONS_KEY, JSON.stringify(pins));
}

/**
 * Local reading preference: which sessions lead the sidebar. Pins live in
 * `localStorage`, not the product store — they are per-machine reading order,
 * not shared session state, so the two can never overwrite each other.
 */
export function useSessionPins(liveSessionIds: readonly string[]): {
  pins: SessionPin[];
  togglePin: (sessionId: string) => void;
  isPinned: (sessionId: string) => boolean;
} {
  const [pins, setPins] = useState<SessionPin[]>(() => readStoredPins());
  const liveKey = useMemo(() => liveSessionIds.join("\u0000"), [liveSessionIds]);

  useEffect(() => {
    const live = liveKey ? liveKey.split("\u0000") : [];
    setPins((current) => {
      const pruned = prunePins(current, live);
      if (pruned.length === current.length) {
        return current;
      }
      writeStoredPins(pruned);
      return pruned;
    });
    // The joined key keeps the effect off every unrelated re-render.
  }, [liveKey]);

  const togglePin = useCallback((sessionId: string) => {
    setPins((current) => {
      if (current.some((pin) => pin.sessionId === sessionId)) {
        const next = current.filter((pin) => pin.sessionId !== sessionId);
        writeStoredPins(next);
        return next;
      }
      const next = [
        ...current,
        { sessionId, pinnedAt: nextPinTimestamp(current, Date.now()) },
      ];
      writeStoredPins(next);
      return next;
    });
  }, []);

  const pinnedSet = useMemo(
    () => new Set(pins.map((pin) => pin.sessionId)),
    [pins],
  );
  const isPinned = useCallback(
    (sessionId: string) => pinnedSet.has(sessionId),
    [pinnedSet],
  );

  return { pins, togglePin, isPinned };
}
