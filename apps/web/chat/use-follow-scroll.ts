"use client";

import { useCallback, useEffect, useRef, useState } from "react";

import {
  FOLLOW_SCROLL_BEHAVIOR,
  TRANSCRIPT_REPIN_THRESHOLD_PX,
  TRANSCRIPT_SCROLL_ROUNDING_TOLERANCE_PX,
  bottomScrollTop,
  isRecentScrollGesture,
  jumpScrollBehavior,
  reduceTranscriptScroll,
} from "./transcript-scroll";

/**
 * Follow-scroll for the transcript, ported from PI-Desktop's `useFollowScroll`.
 *
 * Explicit follow state is kept apart from the near-bottom visual threshold, and
 * only a real upward gesture releases it (see `transcript-scroll`). The hook owns
 * the state machine and the scroll primitives; the caller owns the
 * ResizeObserver, because it also has to preserve the reading position when older
 * turns are prepended — that correction must happen before any follow.
 */
const GESTURE_EVENTS = ["wheel", "touchmove", "keydown", "pointerdown"] as const;

export function useFollowScroll(): {
  scrollRef: React.RefObject<HTMLDivElement | null>;
  showJump: boolean;
  handleScroll: () => void;
  jumpToLatest: () => void;
  /** Re-pin instantly (no animation), for mount and session switches. */
  pinToLatest: () => void;
  /** Called by the ResizeObserver so growth never paints an unpinned frame. */
  followNow: () => void;
  /** Prepending content adjusts `scrollTop`; tell the state machine where it landed. */
  recordScrollPosition: (top: number) => void;
  /** Leave follow mode deliberately (a jump to an older turn is not a clamp). */
  releaseFollow: () => void;
  /** Scroll a marked turn into view and stay released until the reader returns. */
  jumpToMarker: (messageId: string) => void;
  /** Read the explicit follow state, for a per-session view snapshot. */
  readFollowState: () => { pinned: boolean; scrollTop: number };
  /** Reinstate a captured follow state; the caller writes the position back. */
  restoreFollowState: (state: { pinned: boolean; scrollTop: number }) => void;
  /** Re-derive follow from where a restored position actually landed. */
  settleRestoredFollow: () => void;
} {
  const scrollRef = useRef<HTMLDivElement>(null);
  const pinnedRef = useRef(true);
  const lastScrollTopRef = useRef(0);
  const lastGestureAtRef = useRef(-Infinity);
  const frameRef = useRef(0);
  const [showJump, setShowJump] = useState(false);

  const scrollToBottom = useCallback(
    (behavior: ScrollBehavior = FOLLOW_SCROLL_BEHAVIOR) => {
      const element = scrollRef.current;
      if (!element) {
        return;
      }
      element.scrollTo({
        top: bottomScrollTop(element.scrollHeight, element.clientHeight),
        behavior,
      });
      // Record where the scroller actually landed, not what was asked for: a
      // fractional device pixel ratio lands a fraction away, and the intended value
      // would make the following native scroll event read as a user scrolling up.
      if (behavior !== "smooth") {
        lastScrollTopRef.current = element.scrollTop;
      }
    },
    [],
  );

  const cancelFollow = useCallback(() => {
    cancelAnimationFrame(frameRef.current);
    frameRef.current = 0;
  }, []);

  const recordScrollPosition = useCallback((top: number) => {
    lastScrollTopRef.current = top;
  }, []);

  const scheduleFollowScroll = useCallback(() => {
    if (!pinnedRef.current || frameRef.current !== 0) {
      return;
    }
    frameRef.current = requestAnimationFrame(() => {
      frameRef.current = 0;
      if (pinnedRef.current) {
        scrollToBottom();
      }
    });
  }, [scrollToBottom]);

  const followNow = useCallback(() => {
    if (!pinnedRef.current) {
      return;
    }
    cancelFollow();
    scrollToBottom();
  }, [cancelFollow, scrollToBottom]);

  const pinToLatest = useCallback(() => {
    pinnedRef.current = true;
    setShowJump(false);
    scrollToBottom();
  }, [scrollToBottom]);

  const jumpToLatest = useCallback(() => {
    pinnedRef.current = true;
    setShowJump(false);
    // Motion is optional; arrival is not. A reader who asked for reduced motion
    // still gets the end of the transcript, without the travel.
    scrollToBottom(
      jumpScrollBehavior(
        window.matchMedia("(prefers-reduced-motion: reduce)").matches,
      ),
    );
  }, [scrollToBottom]);

  useEffect(() => cancelFollow, [cancelFollow]);

  const releaseFollow = useCallback(() => {
    cancelFollow();
    pinnedRef.current = false;
    setShowJump(true);
  }, [cancelFollow]);

  const readFollowState = useCallback(
    () => ({ pinned: pinnedRef.current, scrollTop: lastScrollTopRef.current }),
    [],
  );

  /**
   * Restoring a snapshot is not a gesture: the refs are written first so the
   * scroll event the restored position produces reads as no movement, and the
   * jump control is derived from the captured state rather than carried over.
   */
  const restoreFollowState = useCallback(
    (state: { pinned: boolean; scrollTop: number }) => {
      cancelFollow();
      pinnedRef.current = state.pinned;
      lastScrollTopRef.current = state.scrollTop;
      setShowJump(!state.pinned);
    },
    [cancelFollow],
  );

  /**
   * The browser can clamp a restored position — the content it was measured
   * against may not be laid out yet. Follow is then re-derived from where the
   * scroller actually landed: a snapshot whose viewport is the bottom now has
   * nothing left to return to.
   */
  const settleRestoredFollow = useCallback(() => {
    const element = scrollRef.current;
    if (!element) {
      return;
    }
    const distanceFromBottom = Math.max(
      0,
      element.scrollHeight - element.clientHeight - element.scrollTop,
    );
    lastScrollTopRef.current = element.scrollTop;
    if (distanceFromBottom < TRANSCRIPT_REPIN_THRESHOLD_PX) {
      pinnedRef.current = true;
      setShowJump(false);
      return;
    }
    setShowJump(!pinnedRef.current);
  }, []);

  /**
   * Marker navigation is a deliberate reading move, so follow is released and
   * stays released. The scroll it produces carries no user gesture at its end
   * (a smooth jump outlives the 200ms gesture window), so the landing position is
   * recorded for the state machine instead of being read as a layout clamp.
   */
  const jumpToMarker = useCallback(
    (messageId: string) => {
      const scroller = scrollRef.current;
      const target = scroller?.querySelector<HTMLElement>(
        `[data-message-id="${CSS.escape(messageId)}"]`,
      );
      if (!scroller || !target) {
        return;
      }
      releaseFollow();
      target.scrollIntoView({
        block: "start",
        behavior: jumpScrollBehavior(
          window.matchMedia("(prefers-reduced-motion: reduce)").matches,
        ),
      });
      requestAnimationFrame(() => recordScrollPosition(scroller.scrollTop));
    },
    [recordScrollPosition, releaseFollow],
  );

  // A real scroll-up always follows a wheel/touch/key/pointer input within a
  // frame or two; programmatic follow scrolling and layout clamps do not.
  useEffect(() => {
    const markGesture = () => {
      lastGestureAtRef.current = performance.now();
    };
    for (const type of GESTURE_EVENTS) {
      window.addEventListener(type, markGesture, { passive: true });
    }
    return () => {
      for (const type of GESTURE_EVENTS) {
        window.removeEventListener(type, markGesture);
      }
    };
  }, []);

  const handleScroll = useCallback(() => {
    const element = scrollRef.current;
    if (!element) {
      return;
    }
    const wasPinned = pinnedRef.current;
    const gesturing = isRecentScrollGesture(
      performance.now(),
      lastGestureAtRef.current,
    );
    const transition = reduceTranscriptScroll({
      previousScrollTop: lastScrollTopRef.current,
      scrollTop: element.scrollTop,
      scrollHeight: element.scrollHeight,
      clientHeight: element.clientHeight,
      wasPinned,
      tolerancePx: gesturing ? 0 : TRANSCRIPT_SCROLL_ROUNDING_TOLERANCE_PX,
    });
    lastScrollTopRef.current = element.scrollTop;
    if (transition.releasedFollow) {
      cancelFollow();
    }
    if (gesturing && transition.releasedFollow) {
      pinnedRef.current = false;
      setShowJump(true);
    } else if (transition.releasedFollow) {
      pinnedRef.current = wasPinned;
      setShowJump(!wasPinned);
      scheduleFollowScroll();
    } else {
      pinnedRef.current = transition.pinned;
      setShowJump(transition.showJump);
    }
  }, [cancelFollow, scheduleFollowScroll]);

  return {
    scrollRef,
    showJump,
    handleScroll,
    jumpToLatest,
    pinToLatest,
    followNow,
    recordScrollPosition,
    releaseFollow,
    jumpToMarker,
    readFollowState,
    restoreFollowState,
    settleRestoredFollow,
  };
}
