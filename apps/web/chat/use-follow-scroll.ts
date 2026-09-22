"use client";

import { useCallback, useEffect, useRef, useState } from "react";

import {
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
} {
  const scrollRef = useRef<HTMLDivElement>(null);
  const pinnedRef = useRef(true);
  const lastScrollTopRef = useRef(0);
  const lastGestureAtRef = useRef(-Infinity);
  const frameRef = useRef(0);
  const [showJump, setShowJump] = useState(false);

  const scrollToBottom = useCallback((behavior: ScrollBehavior = "auto") => {
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
    if (behavior === "auto") {
      lastScrollTopRef.current = element.scrollTop;
    }
  }, []);

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
  };
}
