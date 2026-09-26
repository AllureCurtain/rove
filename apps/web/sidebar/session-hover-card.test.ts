import { describe, expect, it } from "vitest";

import {
  SESSION_HOVER_CARD_GAP_PX,
  SESSION_HOVER_CARD_IDLE,
  SESSION_HOVER_CARD_VIEWPORT_MARGIN_PX,
  reduceSessionHoverCard,
  placeSessionHoverCard,
  readSessionHoverCardAnchor,
  sessionHoverPointerAvailable,
  type SessionHoverCardState,
} from "./session-hover-card";

function run(events: Parameters<typeof reduceSessionHoverCard>[1][]): SessionHoverCardState {
  return events.reduce(reduceSessionHoverCard, SESSION_HOVER_CARD_IDLE);
}

describe("session hover card state", () => {
  it("stays closed while the delay is pending", () => {
    expect(run([{ type: "hover", sessionId: "s1" }])).toEqual({
      pending: "s1",
      open: null,
      suppressed: false,
    });
  });

  it("opens when the delay elapses for the hovered session", () => {
    expect(run([{ type: "hover", sessionId: "s1" }, { type: "delay", sessionId: "s1" }])).toEqual(
      { pending: null, open: "s1", suppressed: false },
    );
  });

  it("cancels a pending card when the pointer leaves early", () => {
    expect(run([{ type: "hover", sessionId: "s1" }, { type: "leave" }])).toEqual(
      SESSION_HOVER_CARD_IDLE,
    );
  });

  it("ignores a delay that arrives after the pointer moved to another row", () => {
    const state = run([
      { type: "hover", sessionId: "s1" },
      { type: "leave" },
      { type: "hover", sessionId: "s2" },
      { type: "delay", sessionId: "s1" },
    ]);
    expect(state).toEqual({ pending: "s2", open: null, suppressed: false });
  });

  it("shows the newest row when the pointer moves between rows", () => {
    expect(
      run([
        { type: "hover", sessionId: "s1" },
        { type: "delay", sessionId: "s1" },
        { type: "hover", sessionId: "s2" },
        { type: "delay", sessionId: "s2" },
      ]),
    ).toEqual({ pending: null, open: "s2", suppressed: false });
  });

  it("closes on Escape and keeps it closed until the pointer leaves", () => {
    const dismissed = run([
      { type: "hover", sessionId: "s1" },
      { type: "delay", sessionId: "s1" },
      { type: "dismiss" },
    ]);
    expect(dismissed).toEqual({ pending: null, open: null, suppressed: true });

    // Still hovering the same row: neither a re-entry event nor a stale timer
    // may reopen what Escape closed.
    expect(reduceSessionHoverCard(dismissed, { type: "hover", sessionId: "s1" })).toEqual(
      dismissed,
    );
    expect(reduceSessionHoverCard(dismissed, { type: "delay", sessionId: "s1" })).toEqual(
      dismissed,
    );

    // Leaving clears the suppression, so hovering again works.
    const left = reduceSessionHoverCard(dismissed, { type: "leave" });
    const rehovered = reduceSessionHoverCard(left, { type: "hover", sessionId: "s1" });
    expect(reduceSessionHoverCard(rehovered, { type: "delay", sessionId: "s1" })).toEqual({
      pending: null,
      open: "s1",
      suppressed: false,
    });
  });

  it("treats Escape with nothing to close as a no-op", () => {
    expect(run([{ type: "dismiss" }])).toEqual(SESSION_HOVER_CARD_IDLE);
  });

  it("keeps an open card while the pointer briefly leaves and returns", () => {
    const state = run([
      { type: "hover", sessionId: "s1" },
      { type: "delay", sessionId: "s1" },
      { type: "leave" },
      { type: "hover", sessionId: "s1" },
    ]);
    // Leaving closes immediately (design: the card disappears when the pointer
    // moves away), and returning re-arms the delay rather than showing at once.
    expect(state).toEqual({ pending: "s1", open: null, suppressed: false });
  });
});

describe("session hover card placement", () => {
  const viewport = { width: 1280, height: 800 };
  const size = { width: 320, height: 200 };

  it("sits to the right of the row", () => {
    expect(placeSessionHoverCard({ top: 120, right: 280 }, viewport, size)).toEqual({
      top: 120,
      left: 280 + SESSION_HOVER_CARD_GAP_PX,
    });
  });

  it("stays inside a narrow viewport", () => {
    const placed = placeSessionHoverCard({ top: 120, right: 640 }, { width: 700, height: 800 }, size);
    expect(placed.left).toBe(700 - 320 - SESSION_HOVER_CARD_VIEWPORT_MARGIN_PX);
    expect(placed.left).toBeGreaterThanOrEqual(SESSION_HOVER_CARD_VIEWPORT_MARGIN_PX);
  });

  it("pulls a card near the bottom back into view", () => {
    expect(placeSessionHoverCard({ top: 780, right: 280 }, viewport, size).top).toBe(
      800 - 200 - SESSION_HOVER_CARD_VIEWPORT_MARGIN_PX,
    );
  });

  it("never places a card above the top edge", () => {
    expect(placeSessionHoverCard({ top: -40, right: 280 }, viewport, size).top).toBe(
      SESSION_HOVER_CARD_VIEWPORT_MARGIN_PX,
    );
  });
});

describe("session hover pointer", () => {
  it("requires a matching media query", () => {
    const asked: string[] = [];
    const available = sessionHoverPointerAvailable((query) => {
      asked.push(query);
      return { matches: true };
    });
    expect(available).toBe(true);
    expect(asked).toEqual(["(hover: hover) and (pointer: fine)"]);
  });

  it("is unavailable where the query does not match", () => {
    expect(sessionHoverPointerAvailable(() => ({ matches: false }))).toBe(false);
  });

  it("is unavailable without matchMedia", () => {
    expect(sessionHoverPointerAvailable(undefined)).toBe(false);
  });
});

describe("session hover card anchor", () => {
  it("reads the row's top and right edge", () => {
    expect(
      readSessionHoverCardAnchor({ getBoundingClientRect: () => ({ top: 32, right: 240 }) }),
    ).toEqual({ top: 32, right: 240 });
  });

  it("reports no anchor without a row", () => {
    expect(readSessionHoverCardAnchor(null)).toBeNull();
  });
});
