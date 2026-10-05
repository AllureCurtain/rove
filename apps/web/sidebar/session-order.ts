/**
 * Session ordering for the sidebar.
 *
 * Pinned sessions lead, newest pin first. Pins made within the same
 * millisecond get distinct timestamps so their order is stable. A collapsed
 * sidebar never hides a pin, and the open session is always reachable.
 */

export interface OrderableSession {
  id: string;
  modifiedAt: number;
}

export interface SessionPin {
  sessionId: string;
  pinnedAt: number;
}

export interface SessionOrdering<T extends OrderableSession> {
  ordered: T[];
  /** How many of `ordered` stay visible while the list is collapsed. */
  visibleWhenCollapsed: number;
}

export function nextPinTimestamp(pins: readonly SessionPin[], now: number): number {
  const latest = pins.reduce((max, pin) => Math.max(max, pin.pinnedAt), 0);
  return Math.max(now, latest + 1);
}

export function orderSessions<T extends OrderableSession>(
  sessions: readonly T[],
  pins: readonly SessionPin[],
  options: { collapsedLimit: number; activeSessionId: string | null },
): SessionOrdering<T> {
  const pinnedAt = new Map(pins.map((pin) => [pin.sessionId, pin.pinnedAt]));
  const ordered = [...sessions].sort((left, right) => {
    const leftPin = pinnedAt.get(left.id);
    const rightPin = pinnedAt.get(right.id);
    if (leftPin !== undefined && rightPin !== undefined) {
      return rightPin - leftPin;
    }
    if (leftPin !== undefined) {
      return -1;
    }
    if (rightPin !== undefined) {
      return 1;
    }
    return right.modifiedAt - left.modifiedAt;
  });

  const pinnedCount = ordered.filter((session) => pinnedAt.has(session.id)).length;
  let visible = Math.max(options.collapsedLimit, pinnedCount);
  if (options.activeSessionId) {
    const activeIndex = ordered.findIndex((session) => session.id === options.activeSessionId);
    if (activeIndex >= visible) {
      visible = activeIndex + 1;
    }
  }
  return { ordered, visibleWhenCollapsed: Math.min(visible, ordered.length) };
}

/** Drop pins whose session no longer exists, so removed sessions leave no trace. */
export function prunePins(pins: readonly SessionPin[], sessionIds: readonly string[]): SessionPin[] {
  const live = new Set(sessionIds);
  return pins.filter((pin) => live.has(pin.sessionId));
}
