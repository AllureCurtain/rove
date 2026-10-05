/**
 * Transcript follow-scroll reducer.
 *
 * The shell had a single `atLatest` flag recomputed from a 48px threshold on
 * every scroll event, which cannot tell a user gesture from a layout clamp: a
 * fractional device pixel ratio makes the browser report a position a fraction of
 * a pixel away from what a correction asked for, and compared strictly that
 * fraction reads as the user scrolling up and drops follow mode. *Explicit*
 * follow state is therefore kept separate from the near-bottom visual threshold,
 * and only a real upward movement releases it.
 */

/** Distance from the bottom that counts as "at the latest". */
export const TRANSCRIPT_REPIN_THRESHOLD_PX = 48;

/** Sub-pixel slack for scroll events no user input produced. */
export const TRANSCRIPT_SCROLL_ROUNDING_TOLERANCE_PX = 1;

/**
 * A real gesture precedes its scroll events by at most a frame or two and keeps
 * firing while it lasts; programmatic follow scrolling and layout clamps emit
 * scroll events with no preceding input.
 */
export const TRANSCRIPT_SCROLL_GESTURE_WINDOW_MS = 200;

export interface TranscriptScrollInput {
  previousScrollTop: number;
  scrollTop: number;
  scrollHeight: number;
  clientHeight: number;
  wasPinned: boolean;
  /** Movement under this many pixels is not movement. */
  tolerancePx?: number;
}

export interface TranscriptScrollTransition {
  distanceFromBottom: number;
  movedUp: boolean;
  movedDown: boolean;
  releasedFollow: boolean;
  pinned: boolean;
  showJump: boolean;
}

export function reduceTranscriptScroll({
  previousScrollTop,
  scrollTop,
  scrollHeight,
  clientHeight,
  wasPinned,
  tolerancePx = 0,
}: TranscriptScrollInput): TranscriptScrollTransition {
  const distanceFromBottom = Math.max(
    0,
    scrollHeight - scrollTop - clientHeight,
  );
  const movedUp = scrollTop < previousScrollTop - tolerancePx;
  const movedDown = scrollTop > previousScrollTop + tolerancePx;
  const nearBottom = distanceFromBottom < TRANSCRIPT_REPIN_THRESHOLD_PX;
  const releasedFollow = movedUp && distanceFromBottom > 0;
  const pinned = releasedFollow ? false : wasPinned || (movedDown && nearBottom);

  return {
    distanceFromBottom,
    movedUp,
    movedDown,
    releasedFollow,
    pinned,
    showJump: !pinned,
  };
}

export function isRecentScrollGesture(
  now: number,
  lastGestureAt: number,
): boolean {
  return now - lastGestureAt <= TRANSCRIPT_SCROLL_GESTURE_WINDOW_MS;
}

/** Where a pinned scroller wants to sit; fractional positions are handled by the caller. */
export function bottomScrollTop(scrollHeight: number, clientHeight: number): number {
  return Math.max(0, scrollHeight - clientHeight);
}

export interface AnchoredScrollInput {
  /** The anchored node's viewport top, after the content changed. */
  anchorTop: number;
  /** The scroller's viewport top, after the content changed. */
  scrollerTop: number;
  /** The node's top relative to the scroller's top, before the change. */
  anchorOffset: number;
}

/**
 * How far a scroller has to move for the node the reader is on to keep the
 * viewport offset it had.
 *
 * `scrollHeight` cannot answer this once revealing older runs also releases
 * newest ones: its delta is what arrived above minus what left below, and only
 * what arrived above moves the reader. Measuring a node that survives the change
 * is what tells the two apart.
 */
export function anchoredScrollDelta({
  anchorTop,
  scrollerTop,
  anchorOffset,
}: AnchoredScrollInput): number {
  return anchorTop - scrollerTop - anchorOffset;
}

/**
 * Follow must not animate. `.chat-transcript` sets `scroll-behavior: smooth`, and
 * with it a `behavior: "auto"` scroll is *animated*: the next content growth then
 * re-targets the animation, which settles short of the bottom (measured 22px at
 * 1280x800). An explicit `instant` overrides the stylesheet, so following is a
 * move rather than a journey — only the jump control is allowed to travel.
 */
export const FOLLOW_SCROLL_BEHAVIOR: ScrollBehavior = "instant";

/** Scroll behaviour for a jump control: motion is optional, arrival is not. */
export function jumpScrollBehavior(reducedMotion: boolean): ScrollBehavior {
  return reducedMotion ? "instant" : "smooth";
}
