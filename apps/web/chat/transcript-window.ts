/**
 * Two-layer window over a long transcript.
 *
 * "Loaded" counts the runs the server has already returned; "mounted" counts
 * the runs actually attached to the DOM. Growing mounts runs that are already
 * in memory first — network latency must never express itself as scroll jank —
 * and only a request decision is offered for the server page once everything
 * loaded is on screen and the server published a cursor for an older page.
 */

export const INITIAL_MOUNTED = 12;
export const MOUNT_STEP = 16;

export interface TranscriptWindow {
  /** Runs attached to the DOM. */
  mounted: number;
  /** Runs the server has returned so far. */
  loaded: number;
}

export type TranscriptWindowEvent =
  | { type: "grow" }
  | { type: "loaded"; count: number };

export function initialTranscriptWindow(loaded: number): TranscriptWindow {
  return {
    mounted: Math.min(INITIAL_MOUNTED, loaded),
    loaded,
  };
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
    case "loaded":
      return { ...window, loaded: window.loaded + event.count };
  }
}

/**
 * Whether the older page has to come from the server. Everything already
 * loaded must be mounted first, and the server must have published a cursor.
 */
export function shouldRequestOlderPage(
  window: TranscriptWindow,
  cursor: string | number | null | undefined,
): boolean {
  return window.mounted >= window.loaded && cursor !== null && cursor !== undefined;
}

/** How many runs are in memory but not yet on screen. */
export function hiddenMountedCount(window: TranscriptWindow): number {
  return Math.max(0, Math.min(window.loaded, window.loaded) - window.mounted);
}
