import { describe, expect, it } from "vitest";

import {
  SIDEBAR_OVERLAY_CLOSE_DELAY_MS,
  shouldFocusSessionHeadingAfterSelection,
  shouldShowSidebarHoverZone,
} from "./sidebar-overlay";

describe("narrow-screen rail overlay", () => {
  it("summons the rail from the edge only while it is shut", () => {
    expect(shouldShowSidebarHoverZone({ narrow: true, open: false })).toBe(true);
    expect(shouldShowSidebarHoverZone({ narrow: true, open: true })).toBe(false);
    expect(shouldShowSidebarHoverZone({ narrow: false, open: false })).toBe(false);
  });

  it("keeps a short grace period so a gap crossing is not a flicker", () => {
    // The reference uses 120ms; the value is part of the behaviour, so pin it.
    expect(SIDEBAR_OVERLAY_CLOSE_DELAY_MS).toBe(120);
    expect(SIDEBAR_OVERLAY_CLOSE_DELAY_MS).toBeGreaterThan(0);
    expect(SIDEBAR_OVERLAY_CLOSE_DELAY_MS).toBeLessThan(500);
  });

  it("only a deliberate open returns focus to the conversation", () => {
    expect(shouldFocusSessionHeadingAfterSelection({ deliberate: true })).toBe(true);
    expect(shouldFocusSessionHeadingAfterSelection({ deliberate: false })).toBe(false);
  });
});
