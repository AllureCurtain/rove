import type { TranscriptRunGroup } from "../lib/rove-state";
import type { DisclosureState } from "./disclosure";
import type { TranscriptWindow } from "./transcript-window";

/**
 * Per-session view snapshot for session switches.
 *
 * Switching sessions replaces the transcript data, not its DOM: the transcript
 * is not remounted, so the reader's viewport, mount window and disclosure
 * choices would otherwise be dropped and the next timeline would open at the
 * tail again. This module owns the capture/restore rules as pure functions plus
 * a bounded store; the shell only calls it at the right moments.
 *
 * Identity is a fingerprint of the run set (ordinal plus last observed event
 * sequence). A session that advanced while it was in the background no longer
 * matches its snapshot: the disclosure map is still restored by entry id, but
 * the viewport is not, because the text underneath it moved.
 */

export const SESSION_VIEW_SNAPSHOT_CAPACITY = 8;

export interface SessionViewCapture {
  scrollTop: number;
  followPinned: boolean;
  window: TranscriptWindow;
  disclosure: Map<string, DisclosureState>;
}

export interface SessionViewSnapshot extends SessionViewCapture {
  fingerprint: string;
}

export interface SessionViewRestore {
  /** The session advanced since capture: keep the live viewport and go to the tail. */
  stale: boolean;
  /** Captured mount window, or null when the snapshot is stale. */
  window: TranscriptWindow | null;
  /** Disclosure survives a stale snapshot: entries are matched by id. */
  disclosure: Map<string, DisclosureState>;
  /** Viewport to reinstate, or null when the snapshot is stale. */
  scrollTop: number | null;
  followPinned: boolean;
}

/**
 * The run set as it was seen: one `identity:lastEventSeq` pair per run, in
 * timeline order. Event sequence alone is not enough — a replaced run keeps its
 * ordinal but restarts its sequence — and ordinals are only unique per session.
 */
export function transcriptFingerprint(groups: TranscriptRunGroup[]): string {
  return groups
    .map((group) => {
      let lastSeq = 0;
      for (const item of group.items) {
        const seq = item.entry.eventSeq;
        if (typeof seq === "number" && Number.isFinite(seq) && seq > lastSeq) {
          lastSeq = seq;
        }
      }
      const identity = group.runOrdinal ?? group.runId ?? "current";
      return `${identity}:${lastSeq}`;
    })
    .join("|");
}

/**
 * Compare a snapshot with the session as it stands now. Only a matching
 * fingerprint restores the viewport; disclosure is restored either way.
 */
export function restoreSessionView(
  snapshot: SessionViewSnapshot,
  liveFingerprint: string,
): SessionViewRestore {
  const stale = snapshot.fingerprint !== liveFingerprint;
  return {
    stale,
    window: stale ? null : snapshot.window,
    disclosure: new Map(snapshot.disclosure),
    scrollTop: stale ? null : snapshot.scrollTop,
    followPinned: stale ? false : snapshot.followPinned,
  };
}

export interface SessionViewSnapshotStore {
  /** Store the view of one session, evicting the least recently captured. */
  record(sessionId: string, capture: SessionViewCapture, fingerprint: string): void;
  /** Read without consuming, for decisions taken before the data arrives. */
  peek(sessionId: string): SessionViewSnapshot | null;
  /** Consume: a snapshot is applied at most once. */
  take(sessionId: string): SessionViewSnapshot | null;
  forget(sessionId: string): void;
  clear(): void;
  size(): number;
  /** Oldest first, for tests and diagnostics. */
  keys(): string[];
}

export function createSessionViewSnapshotStore(
  capacity: number = SESSION_VIEW_SNAPSHOT_CAPACITY,
): SessionViewSnapshotStore {
  const entries = new Map<string, SessionViewSnapshot>();
  return {
    record(sessionId, capture, fingerprint) {
      // Re-inserting moves the session to the most-recent end of the Map order.
      entries.delete(sessionId);
      entries.set(sessionId, {
        ...capture,
        disclosure: new Map(capture.disclosure),
        fingerprint,
      });
      while (entries.size > capacity) {
        const oldest = entries.keys().next().value;
        if (oldest === undefined) {
          break;
        }
        entries.delete(oldest);
      }
    },
    peek(sessionId) {
      return entries.get(sessionId) ?? null;
    },
    take(sessionId) {
      const snapshot = entries.get(sessionId) ?? null;
      entries.delete(sessionId);
      return snapshot;
    },
    forget(sessionId) {
      entries.delete(sessionId);
    },
    clear() {
      entries.clear();
    },
    size() {
      return entries.size;
    },
    keys() {
      return [...entries.keys()];
    },
  };
}

/** The shell's single transcript: one store, one registered capture. */
export const sessionViewSnapshots = createSessionViewSnapshotStore();

let activeCapture: (() => void) | null = null;

/**
 * The transcript registers how to read its current view. Capture is driven by
 * the data layer, which is the side that knows a reset is about to happen —
 * leaving the session, or replacing it with another one.
 */
export function registerSessionViewCapture(capture: (() => void) | null): void {
  activeCapture = capture;
}

export function captureActiveSessionView(): void {
  activeCapture?.();
}
