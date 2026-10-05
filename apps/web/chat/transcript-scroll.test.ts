import { describe, expect, it } from "vitest";

import {
  FOLLOW_SCROLL_BEHAVIOR,
  TRANSCRIPT_REPIN_THRESHOLD_PX,
  TRANSCRIPT_SCROLL_GESTURE_WINDOW_MS,
  TRANSCRIPT_SCROLL_ROUNDING_TOLERANCE_PX,
  anchoredScrollDelta,
  bottomScrollTop,
  isRecentScrollGesture,
  jumpScrollBehavior,
  reduceTranscriptScroll,
} from "./transcript-scroll";

const base = {
  previousScrollTop: 1000,
  scrollTop: 1000,
  scrollHeight: 2000,
  clientHeight: 500,
  wasPinned: true,
};

describe("transcript follow-scroll", () => {
  it("stays pinned while the scroller sits at the bottom", () => {
    const idle = reduceTranscriptScroll({ ...base, scrollTop: 1500 });
    expect(idle.distanceFromBottom).toBe(0);
    expect(idle.pinned).toBe(true);
    expect(idle.showJump).toBe(false);
  });

  it("releases follow only for a real upward move", () => {
    const up = reduceTranscriptScroll({
      ...base,
      previousScrollTop: 1500,
      scrollTop: 1400,
    });
    expect(up.movedUp).toBe(true);
    expect(up.releasedFollow).toBe(true);
    expect(up.pinned).toBe(false);
    expect(up.showJump).toBe(true);
  });

  it("ignores a sub-pixel clamp that no input produced", () => {
    // A fractional device pixel ratio lands a fraction of a pixel away from what
    // a layout correction asked for; compared strictly that reads as a user
    // scroll and would drop follow mode.
    const clamp = reduceTranscriptScroll({
      ...base,
      previousScrollTop: 1500,
      scrollTop: 1500 - TRANSCRIPT_SCROLL_ROUNDING_TOLERANCE_PX,
      tolerancePx: TRANSCRIPT_SCROLL_ROUNDING_TOLERANCE_PX,
    });
    expect(clamp.movedUp).toBe(false);
    expect(clamp.releasedFollow).toBe(false);
    expect(clamp.pinned).toBe(true);
  });

  it("still releases for a one-pixel gesture, which passes tolerance 0", () => {
    const gesture = reduceTranscriptScroll({
      ...base,
      previousScrollTop: 1500,
      scrollTop: 1499,
      tolerancePx: 0,
    });
    expect(gesture.releasedFollow).toBe(true);
  });

  it("does not release when an upward move is clamped at the very bottom", () => {
    // Prepending older turns can push content down while the scroller is already
    // at the bottom; `distanceFromBottom > 0` keeps that from unpinning.
    const clamp = reduceTranscriptScroll({
      ...base,
      previousScrollTop: 0,
      scrollTop: 0,
      scrollHeight: base.clientHeight,
      wasPinned: false,
    });
    expect(clamp.releasedFollow).toBe(false);
  });

  it("re-pins when the user scrolls back down near the bottom", () => {
    // 2000 - 1460 - 500 = 40px from the bottom, inside the 48px repin threshold.
    const back = reduceTranscriptScroll({
      ...base,
      previousScrollTop: 1400,
      scrollTop: 1460,
      wasPinned: false,
    });
    expect(back.distanceFromBottom).toBe(40);
    expect(back.movedDown).toBe(true);
    expect(back.pinned).toBe(true);
    expect(back.showJump).toBe(false);
  });

  it("stays unpinned when scrolling down away from the bottom", () => {
    const away = reduceTranscriptScroll({
      ...base,
      previousScrollTop: 500,
      scrollTop: 600,
      wasPinned: false,
    });
    expect(away.pinned).toBe(false);
    expect(away.showJump).toBe(true);
  });

  it("uses the documented threshold and gesture window", () => {
    expect(TRANSCRIPT_REPIN_THRESHOLD_PX).toBe(48);
    expect(TRANSCRIPT_SCROLL_GESTURE_WINDOW_MS).toBe(200);
    expect(isRecentScrollGesture(1000, 1000 - 200)).toBe(true);
    expect(isRecentScrollGesture(1000, 1000 - 201)).toBe(false);
    expect(isRecentScrollGesture(1000, -Infinity)).toBe(false);
  });

  it("computes the bottom position and respects reduced motion on a jump", () => {
    expect(bottomScrollTop(2000, 500)).toBe(1500);
    expect(bottomScrollTop(100, 500)).toBe(0);
    // The stylesheet says `scroll-behavior: smooth`; follow must override it, or a
    // re-targeted animation settles short of the bottom.
    expect(FOLLOW_SCROLL_BEHAVIOR).toBe("instant");
    expect(jumpScrollBehavior(false)).toBe("smooth");
    expect(jumpScrollBehavior(true)).toBe("instant");
  });
});

describe("transcript anchor correction", () => {
  it("is zero for a node that did not move relative to the scroller", () => {
    expect(
      anchoredScrollDelta({ anchorTop: 120, scrollerTop: 100, anchorOffset: 20 }),
    ).toBe(0);
  });

  it("measures the drift of a node content was inserted above", () => {
    // The node sat 20px below the scroller's top edge and is now 620px below it,
    // because 600px of older runs arrived above: the scroller has to move by the
    // same 600px for the reader to keep looking at it.
    expect(
      anchoredScrollDelta({ anchorTop: 720, scrollerTop: 100, anchorOffset: 20 }),
    ).toBe(600);
  });

  it("does not read content released below the node as movement", () => {
    // A growth step can release exactly as much below the reader as it reveals
    // above, so `scrollHeight` reports no change at all; the node still says the
    // reader has 600px of new content above them, and that is what it returns.
    expect(
      anchoredScrollDelta({ anchorTop: 720, scrollerTop: 100, anchorOffset: 20 }),
    ).toBe(600);
  });

  it("corrects in both directions", () => {
    // Content above the node can also leave the DOM: the node drifts up, and the
    // correction is signed rather than a prepend-only adjustment.
    expect(
      anchoredScrollDelta({ anchorTop: 40, scrollerTop: 100, anchorOffset: 20 }),
    ).toBe(-80);
  });
});
