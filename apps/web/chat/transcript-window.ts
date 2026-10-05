/**
 * Two-layer window over a long transcript.
 *
 * "Loaded" counts the runs the server has already returned; "mounted" counts the
 * runs the reader has revealed from the tail of that loaded layer. Growing
 * reveals older runs that are already in memory first — network latency must
 * never express itself as scroll jank — and only a request decision is offered
 * for the server page once everything loaded has been revealed and the server
 * published a cursor for an older page.
 *
 * The DOM holds at most `MAX_MOUNTED` runs, though. Revealing past that evicts
 * the newest runs instead of growing the document without bound, so a reader who
 * walks up through a long session keeps a window around the place they are
 * reading. `mounted` stays the reveal budget — how far back from the tail the
 * reader has walked — because that is the number the older-history decisions and
 * the per-session view snapshot read.
 */

export const INITIAL_MOUNTED = 12;
export const MOUNT_STEP = 16;

/**
 * How many runs may be attached to the DOM at once.
 *
 * One server page is at most 64 runs (`limit_runs`), so a cap of 64 lets a
 * reader who asked for one more page read everything it returned before the
 * window starts releasing anything, and it is 4x `MOUNT_STEP`, so a growth step
 * adds 16 runs above the reader and releases 16 below while 48 of the runs that
 * were already being read stay on screen.
 */
export const MAX_MOUNTED = 64;

export interface TranscriptWindow {
  /**
   * Runs revealed from the tail: the newest `mounted` loaded runs. Only the
   * newest `MAX_MOUNTED` of them are attached to the DOM.
   */
  mounted: number;
  /** Runs the server has returned so far. */
  loaded: number;
}

export type TranscriptWindowEvent =
  | { type: "grow" }
  /**
   * Mount the run `count` runs back from the tail, for a reveal that must reach
   * a known run rather than walk one step at a time. Clamped to what is loaded,
   * like every other growth: mounting is never allowed to outrun the server.
   */
  | { type: "mount"; count: number }
  | { type: "loaded"; count: number }
  /**
   * Back to the live tail: the newest run is inside the window again. Follow
   * mode and the "return to latest" control both mean the newest run, and a
   * window that released it would have nothing left to follow.
   */
  | { type: "tail" };

export function initialTranscriptWindow(loaded: number): TranscriptWindow {
  return {
    mounted: Math.min(INITIAL_MOUNTED, loaded),
    loaded,
  };
}

/**
 * The slice of a `total`-run timeline the DOM holds. The window is anchored at
 * the older edge the reader revealed and bounded at the newer edge by
 * `MAX_MOUNTED`, so growing toward older history releases the newest runs rather
 * than refusing to grow.
 */
export interface MountedRunRange {
  /** Index of the oldest run attached to the DOM. */
  start: number;
  /** One past the newest run attached to the DOM. */
  end: number;
}

export function mountedRunRange(
  window: TranscriptWindow,
  total: number,
): MountedRunRange {
  const revealed = Math.max(0, Math.min(window.mounted, total));
  const start = total - revealed;
  return { start, end: Math.min(total, start + MAX_MOUNTED) };
}

/**
 * Whether the run `count` runs back from the tail is on the DOM right now. A
 * reveal asks this before it decides to re-anchor: the run can sit inside the
 * revealed range and still have been released from the newest end.
 */
export function mountsRun(window: TranscriptWindow, count: number): boolean {
  return (
    count >= 1 && count <= window.mounted && window.mounted - count < MAX_MOUNTED
  );
}

export function reduceTranscriptWindow(
  window: TranscriptWindow,
  event: TranscriptWindowEvent,
): TranscriptWindow {
  switch (event.type) {
    case "grow": {
      const mounted = Math.min(window.loaded, window.mounted + MOUNT_STEP);
      return { ...window, mounted };
    }
    case "mount": {
      if (mountsRun(window, event.count)) {
        return window;
      }
      // A reveal that only grew its budget could miss its own target: once the
      // reader has revealed more than a window holds, the target can be inside
      // the revealed range and outside the DOM. Pulling the budget back to the
      // cap is the largest budget that always shows a run this new, and the
      // reader's own budget is kept whenever it already shows the target.
      const mounted = Math.min(
        window.loaded,
        Math.max(event.count, Math.min(window.mounted, MAX_MOUNTED)),
      );
      return mounted === window.mounted ? window : { ...window, mounted };
    }
    case "loaded":
      return { ...window, loaded: window.loaded + event.count };
    case "tail": {
      if (window.mounted <= MAX_MOUNTED) {
        return window;
      }
      return { ...window, mounted: MAX_MOUNTED };
    }
  }
}

/**
 * Whether the older page has to come from the server. Everything already
 * loaded must be revealed first, and the server must have published a cursor.
 */
export function shouldRequestOlderPage(
  window: TranscriptWindow,
  cursor: string | number | null | undefined,
): boolean {
  return window.mounted >= window.loaded && cursor !== null && cursor !== undefined;
}

/** How many runs are in memory but not yet revealed. */
export function hiddenMountedCount(window: TranscriptWindow): number {
  return Math.max(0, Math.min(window.loaded, window.loaded) - window.mounted);
}

/**
 * Older history the server has published but the client has not fetched yet.
 * `cursor` is the server's `next_before_ordinal`; it only exists while
 * strictly older runs remain.
 */
export interface OlderHistoryState {
  hasMore: boolean;
  cursor: number | null;
  loading: boolean;
  error: string | null;
}

export const NO_OLDER_HISTORY: OlderHistoryState = {
  hasMore: false,
  cursor: null,
  loading: false,
  error: null,
};

/**
 * What the "older history" affordance should offer right now. Everything
 * already loaded is revealed first, so a server page is only requested once the
 * reveal budget has drained and the server published a cursor.
 */
export type OlderHistoryControl = "none" | "grow" | "server";

export function olderHistoryControl(
  window: TranscriptWindow,
  older: Pick<OlderHistoryState, "hasMore" | "cursor">,
): OlderHistoryControl {
  if (hiddenMountedCount(window) > 0) {
    return "grow";
  }
  if (!older.hasMore || !shouldRequestOlderPage(window, older.cursor)) {
    return "none";
  }
  return "server";
}
