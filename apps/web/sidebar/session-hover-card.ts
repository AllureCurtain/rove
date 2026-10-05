/**
 * Session hover card state (design F4).
 *
 * The card is pure presentation with two policy decisions that are easy to get
 * wrong and cheap to test on their own, so both live here rather than in the
 * component:
 *
 * - **when** the card is allowed to be visible: a hover only arms a delay, and
 *   the card opens when that delay elapses for the session still under the
 *   pointer. Leaving cancels, and Escape closes the card for the rest of the
 *   hover — the pointer never left the row, so re-arming there would reopen
 *   exactly what the reader just dismissed.
 * - **where** it goes: to the right of the row, clamped into the viewport, so a
 *   long sidebar or a short window cannot push it off screen.
 *
 * Timers, media queries, and the portal stay in the component.
 */

export const SESSION_HOVER_CARD_DELAY_MS = 300;
export const SESSION_HOVER_CARD_WIDTH_PX = 320;
export const SESSION_HOVER_CARD_GAP_PX = 8;
export const SESSION_HOVER_CARD_VIEWPORT_MARGIN_PX = 8;

/** Fine pointers only: a touch device has no hover to wait for. */
export const SESSION_HOVER_POINTER_QUERY = "(hover: hover) and (pointer: fine)";

/**
 * The card is portaled out of the row (so the sidebar's scroll clipping cannot
 * cut it off) but stays inside the shell, where the v2 tokens and scoped rules
 * live. Portaling to `document.body` would render an unstyled card.
 */
export const SESSION_HOVER_CARD_SCOPE_SELECTOR = ".product-app-frame";

export interface SessionHoverCardState {
  /** Session whose open delay is running, or null. */
  pending: string | null;
  /** Session whose card is visible, or null. */
  open: string | null;
  /** Escape dismissed the card; only leaving the row clears this. */
  suppressed: boolean;
}

export const SESSION_HOVER_CARD_IDLE: SessionHoverCardState = {
  pending: null,
  open: null,
  suppressed: false,
};

export type SessionHoverCardEvent =
  | { type: "hover"; sessionId: string }
  | { type: "delay"; sessionId: string }
  | { type: "leave" }
  | { type: "dismiss" };

export function reduceSessionHoverCard(
  state: SessionHoverCardState,
  event: SessionHoverCardEvent,
): SessionHoverCardState {
  switch (event.type) {
    case "hover": {
      if (state.suppressed) {
        // The card was dismissed for this hover; the pointer has not left.
        return state;
      }
      if (state.open === event.sessionId) {
        return { pending: null, open: state.open, suppressed: false };
      }
      return { pending: event.sessionId, open: state.open, suppressed: false };
    }
    case "delay": {
      if (state.pending !== event.sessionId || state.suppressed) {
        // The pointer moved on (or the card was dismissed) before the delay
        // elapsed, so this timer no longer speaks for the row. It must not
        // disturb whatever the newer hover put in flight.
        return state;
      }
      return { pending: null, open: event.sessionId, suppressed: false };
    }
    case "leave":
      return SESSION_HOVER_CARD_IDLE;
    case "dismiss":
      if (state.pending === null && state.open === null) {
        return state;
      }
      return { pending: null, open: null, suppressed: true };
    default:
      return state;
  }
}

export interface SessionHoverCardAnchor {
  top: number;
  right: number;
}

export interface SessionHoverCardViewport {
  width: number;
  height: number;
}

export interface SessionHoverCardSize {
  width: number;
  height: number;
}

/**
 * The card opens beside the row and stays inside the viewport. Clamping (rather
 * than flipping to the other side) is deliberate: the sidebar is narrow, so
 * covering it briefly is better than pushing the card under the pointer.
 */
export function placeSessionHoverCard(
  anchor: SessionHoverCardAnchor,
  viewport: SessionHoverCardViewport,
  size: SessionHoverCardSize,
): { top: number; left: number } {
  const margin = SESSION_HOVER_CARD_VIEWPORT_MARGIN_PX;
  const maxLeft = viewport.width - size.width - margin;
  const maxTop = viewport.height - size.height - margin;
  const left = Math.max(margin, Math.min(anchor.right + SESSION_HOVER_CARD_GAP_PX, maxLeft));
  const top = Math.max(margin, Math.min(anchor.top, maxTop));
  return { top, left };
}

export function sessionHoverPointerAvailable(
  matchMedia: ((query: string) => { matches: boolean }) | undefined,
): boolean {
  if (typeof matchMedia !== "function") {
    return false;
  }
  return matchMedia(SESSION_HOVER_POINTER_QUERY).matches;
}

export function readSessionHoverCardAnchor(
  element: { getBoundingClientRect(): { top: number; right: number } } | null,
): SessionHoverCardAnchor | null {
  if (!element) {
    return null;
  }
  const rect = element.getBoundingClientRect();
  return { top: rect.top, right: rect.right };
}
