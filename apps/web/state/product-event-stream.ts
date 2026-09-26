import { PRODUCT_EVENT_KINDS, isStreamCursorExpired } from "../lib/rove-client";

/**
 * Status poll cadence while the directory stream is down. This is the same
 * 2.5s degradation the shell used before the stream existed.
 */
export const PRODUCT_STATUS_FALLBACK_POLL_MS = 2_500;

/**
 * Status poll cadence while the directory stream is live.
 *
 * The stream is the feed; this only bounds how long a fact it cannot express —
 * or a frame that never arrived — can survive. It stays unconditional (one
 * bounded sessions request per workspace) because a demoted poll that also
 * stops when nothing looks busy would have nothing to wake it up again.
 */
export const PRODUCT_STATUS_SAFETY_NET_POLL_MS = 30_000;

export type ProductStatusPoll =
  | { readonly mode: "idle" }
  | { readonly mode: "fallback"; readonly intervalMs: number }
  | { readonly mode: "safety-net"; readonly intervalMs: number };

/**
 * How often the shell should poll the session catalog for status changes.
 *
 * With the stream available, status changes arrive on their own and polling is
 * only a safety net. Without it, polling is the only signal, and the pre-stream
 * guard applies: nothing has to be asked while no session is moving.
 */
export function decideProductStatusPoll({
  streamAvailable,
  hasBusySession,
}: {
  streamAvailable: boolean;
  hasBusySession: boolean;
}): ProductStatusPoll {
  if (streamAvailable) {
    return {
      mode: "safety-net",
      intervalMs: PRODUCT_STATUS_SAFETY_NET_POLL_MS,
    };
  }
  if (hasBusySession) {
    return { mode: "fallback", intervalMs: PRODUCT_STATUS_FALLBACK_POLL_MS };
  }
  return { mode: "idle" };
}

/**
 * The slice of `EventSource` this subscription needs. Narrowing it keeps the
 * wiring testable without a DOM or a network.
 */
export interface ProductEventStreamSource {
  addEventListener(type: string, listener: EventListener): void;
  removeEventListener(type: string, listener: EventListener): void;
  close(): void;
}

/**
 * Wire one product event stream to a catalog refresh.
 *
 * Every delivered frame asks for a refresh regardless of its kind: frames carry
 * status hints only, never the fields the UI renders, so the catalog remains
 * the single source of truth. Availability is reported so the caller can choose
 * a poll cadence, and an expired cursor is reported as itself because the gap it
 * implies is only closed by re-reading the catalog.
 *
 * The returned detach removes the listeners before closing the stream, so a
 * frame from a torn-down effect cannot refresh anything.
 */
export function subscribeToProductEvents(
  source: ProductEventStreamSource,
  {
    refresh,
    onAvailabilityChange,
  }: {
    refresh: () => void;
    onAvailabilityChange: (available: boolean) => void;
  },
): () => void {
  let available = false;
  const setAvailable = (next: boolean) => {
    if (available === next) {
      return;
    }
    available = next;
    onAvailabilityChange(next);
  };

  const handleOpen = () => setAvailable(true);
  const handleFrame = () => {
    // A frame is proof of a working connection even if `open` was missed.
    setAvailable(true);
    refresh();
  };
  const handleError = (event: Event) => {
    setAvailable(false);
    if (isStreamCursorExpired(event)) {
      refresh();
    }
  };

  const listeners: Array<[string, EventListener]> = [
    ["open", handleOpen],
    ["error", handleError],
  ];
  for (const kind of PRODUCT_EVENT_KINDS) {
    listeners.push([kind, handleFrame]);
  }
  for (const [type, listener] of listeners) {
    source.addEventListener(type, listener);
  }

  return () => {
    for (const [type, listener] of listeners) {
      source.removeEventListener(type, listener);
    }
    source.close();
    setAvailable(false);
  };
}
