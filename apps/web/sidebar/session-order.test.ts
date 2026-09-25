import { describe, expect, it } from "vitest";

import { nextPinTimestamp, orderSessions, prunePins, type SessionPin } from "./session-order";

const sessions = [
  { id: "a", modifiedAt: 300 },
  { id: "b", modifiedAt: 200 },
  { id: "c", modifiedAt: 100 },
];

describe("orderSessions", () => {
  it("leads with pins, newest first, then by modification time", () => {
    const pins: SessionPin[] = [
      { sessionId: "c", pinnedAt: 1 },
      { sessionId: "b", pinnedAt: 2 },
    ];
    const ordering = orderSessions(sessions, pins, { collapsedLimit: 1, activeSessionId: null });
    expect(ordering.ordered.map((session) => session.id)).toEqual(["b", "c", "a"]);
  });

  it("keeps every pin visible while collapsed", () => {
    const pins: SessionPin[] = [
      { sessionId: "a", pinnedAt: 1 },
      { sessionId: "b", pinnedAt: 2 },
    ];
    const ordering = orderSessions(sessions, pins, { collapsedLimit: 1, activeSessionId: null });
    expect(ordering.visibleWhenCollapsed).toBe(2);
  });

  it("extends the visible range so the open session is reachable", () => {
    const ordering = orderSessions(sessions, [], { collapsedLimit: 1, activeSessionId: "c" });
    expect(ordering.visibleWhenCollapsed).toBe(3);
  });
});

describe("nextPinTimestamp", () => {
  it("stays strictly above the latest pin even within the same millisecond", () => {
    expect(nextPinTimestamp([{ sessionId: "a", pinnedAt: 5 }], 5)).toBe(6);
  });
});

describe("prunePins", () => {
  it("drops pins for sessions that no longer exist", () => {
    const pins = [{ sessionId: "gone", pinnedAt: 1 }, { sessionId: "a", pinnedAt: 2 }];
    expect(prunePins(pins, ["a"]).map((pin) => pin.sessionId)).toEqual(["a"]);
  });
});
