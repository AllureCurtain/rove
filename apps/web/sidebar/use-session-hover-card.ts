"use client";

import { useCallback, useEffect, useReducer, useRef, useState } from "react";

import {
  SESSION_HOVER_CARD_BRIDGE_MS,
  SESSION_HOVER_CARD_DELAY_MS,
  SESSION_HOVER_CARD_IDLE,
  SESSION_HOVER_CARD_SCOPE_SELECTOR,
  SESSION_HOVER_POINTER_QUERY,
  readSessionHoverCardAnchor,
  reduceSessionHoverCard,
  sessionHoverPointerAvailable,
  type SessionHoverCardAnchor,
} from "./session-hover-card";

export interface SessionHoverCardController {
  /** True when this row's card is on screen. */
  open: boolean;
  /** Element id for the card, referenced by the row's aria-describedby. */
  cardId: string;
  anchorRef: React.RefObject<HTMLButtonElement | null>;
  anchor: SessionHoverCardAnchor | null;
  /** Where the portal renders: the shell frame when the row is inside it. */
  portalHost: HTMLElement | null;
  rowProps: {
    onMouseEnter: () => void;
    onMouseLeave: () => void;
  };
  /**
   * Hover bridge (design §7.3): the card survives the pointer crossing the gap
   * between the row and the card, so its Open action is reachable. Row leave
   * starts a short grace timer that entering the card cancels; leaving the
   * card itself closes it at once.
   */
  cardProps: {
    onMouseEnter: () => void;
    onMouseLeave: () => void;
  };
  dismiss: () => void;
}

/**
 * Drives one session row's hover card (design F4).
 *
 * Deliberately pointer-only: no focus handler, so tabbing the sidebar never
 * opens a card. The card is only rendered after the fine-pointer media query
 * matched on the client, which also keeps the server render free of it.
 */
export function useSessionHoverCard(sessionId: string): SessionHoverCardController {
  const [state, dispatch] = useReducer(reduceSessionHoverCard, SESSION_HOVER_CARD_IDLE);
  const [pointerAvailable, setPointerAvailable] = useState(false);
  const [anchor, setAnchor] = useState<SessionHoverCardAnchor | null>(null);
  const [portalHost, setPortalHost] = useState<HTMLElement | null>(null);
  const anchorRef = useRef<HTMLButtonElement | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const bridgeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const open = state.open === sessionId;

  const clearTimer = useCallback(() => {
    if (timerRef.current !== null) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  }, []);

  const clearBridgeTimer = useCallback(() => {
    if (bridgeTimerRef.current !== null) {
      clearTimeout(bridgeTimerRef.current);
      bridgeTimerRef.current = null;
    }
  }, []);

  useEffect(() => {
    const media = window.matchMedia?.(SESSION_HOVER_POINTER_QUERY);
    const read = () => setPointerAvailable(sessionHoverPointerAvailable(window.matchMedia?.bind(window)));
    read();
    if (!media) {
      return undefined;
    }
    media.addEventListener?.("change", read);
    return () => media.removeEventListener?.("change", read);
  }, []);

  useEffect(
    () => () => {
      clearTimer();
      clearBridgeTimer();
    },
    [clearTimer, clearBridgeTimer],
  );

  // Track the row while the card is visible: the sidebar scrolls, the window
  // resizes, and the row can leave the viewport entirely.
  useEffect(() => {
    if (!open) {
      setAnchor(null);
      setPortalHost(null);
      return undefined;
    }
    const update = () => {
      const element = anchorRef.current;
      const rect = element?.getBoundingClientRect();
      if (rect && (rect.bottom < 0 || rect.top > window.innerHeight)) {
        dispatch({ type: "leave" });
        return;
      }
      setAnchor(readSessionHoverCardAnchor(element));
      setPortalHost(element?.closest<HTMLElement>(SESSION_HOVER_CARD_SCOPE_SELECTOR) ?? document.body);
    };
    update();
    window.addEventListener("scroll", update, true);
    window.addEventListener("resize", update);
    return () => {
      window.removeEventListener("scroll", update, true);
      window.removeEventListener("resize", update);
    };
  }, [open]);

  useEffect(() => {
    if (!open) {
      return undefined;
    }
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        dispatch({ type: "dismiss" });
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [open]);

  const enter = useCallback(() => {
    if (!pointerAvailable) {
      return;
    }
    // Re-entering the row inside the bridge window keeps its card open.
    clearBridgeTimer();
    dispatch({ type: "hover", sessionId });
    clearTimer();
    timerRef.current = setTimeout(() => {
      timerRef.current = null;
      dispatch({ type: "delay", sessionId });
    }, SESSION_HOVER_CARD_DELAY_MS);
  }, [clearTimer, clearBridgeTimer, pointerAvailable, sessionId]);

  const openRef = useRef(open);
  openRef.current = open;

  const leave = useCallback(() => {
    clearTimer();
    clearBridgeTimer();
    if (!openRef.current) {
      // Nothing open to bridge to — the leave is final at once. This matters
      // after Escape: suppression ends when the pointer truly departs, and a
      // delayed dispatch would swallow the next re-enter as "same hover".
      dispatch({ type: "leave" });
      return;
    }
    // Grace before closing: the card sits a few pixels from the row, so the
    // pointer that is plainly heading for it should not watch it vanish mid
    // crossing. Entering the card cancels this; anything else times it out.
    bridgeTimerRef.current = setTimeout(() => {
      bridgeTimerRef.current = null;
      dispatch({ type: "leave" });
    }, SESSION_HOVER_CARD_BRIDGE_MS);
  }, [clearTimer, clearBridgeTimer]);

  const cardEnter = useCallback(() => {
    clearBridgeTimer();
  }, [clearBridgeTimer]);

  const cardLeave = useCallback(() => {
    dispatch({ type: "leave" });
  }, []);

  const dismiss = useCallback(() => {
    clearTimer();
    clearBridgeTimer();
    dispatch({ type: "dismiss" });
  }, [clearTimer, clearBridgeTimer]);

  return {
    open,
    cardId: `session-hover-card-${sessionId}`,
    anchorRef,
    anchor,
    portalHost,
    rowProps: { onMouseEnter: enter, onMouseLeave: leave },
    cardProps: { onMouseEnter: cardEnter, onMouseLeave: cardLeave },
    dismiss,
  };
}
