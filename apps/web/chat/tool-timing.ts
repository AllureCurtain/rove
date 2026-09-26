import type { StreamEvent } from "../lib/rove-types";
import type { WorkbenchAction } from "../lib/rove-state";

/**
 * Where a duration may honestly come from.
 *
 * Canonical tool events carry no timestamps on the wire, so the only wall clock
 * a call can be measured against is the moment its events reached the reducer.
 * That measurement is meaningless for anything that did not arrive live — a
 * restored transcript, or an attach-time snapshot replay — and for one MCP call
 * the runtime already publishes `protocol_metadata.duration_ms`. This module
 * keeps the two apart: arrivals are recorded per tool/input identity, and an
 * activity duration is reported only when every item was observed live.
 */

/** Durations below this are noise, and are not shown at all. */
export const DURATION_BADGE_MIN_MS = 1_000;

export interface ToolArrival {
  /** First live arrival for this identity. */
  first: number;
  /** Most recent live arrival for this identity. */
  last: number;
}

export interface ToolArrivalTracker {
  record(key: string, at: number): void;
  read(key: string): ToolArrival | undefined;
  clear(): void;
  size(): number;
}

export function toolArrivalKey(callId: string): string {
  return `tool:${callId}`;
}

export function inputArrivalKey(inputId: string): string {
  return `input:${inputId}`;
}

export function createToolArrivalTracker(): ToolArrivalTracker {
  const arrivals = new Map<string, ToolArrival>();
  return {
    record(key, at) {
      const existing = arrivals.get(key);
      arrivals.set(
        key,
        existing ? { first: existing.first, last: at } : { first: at, last: at },
      );
    },
    read(key) {
      return arrivals.get(key);
    },
    clear() {
      arrivals.clear();
    },
    size() {
      return arrivals.size;
    },
  };
}

/** The shell has one transcript, so it has one arrival history. */
export const toolArrivals = createToolArrivalTracker();

export interface ActivityTiming {
  /** Ticking elapsed of an activity still in flight, or null when unavailable. */
  liveMs: number | null;
  /** Final duration of a settled activity, or null when unavailable. */
  frozenMs: number | null;
}

const NO_TIMING: ActivityTiming = { liveMs: null, frozenMs: null };

/**
 * An activity duration needs every item to have been seen arrive. One item that
 * only appeared through a replay or a restore makes the start unknown, and a
 * fabricated duration is worse than none.
 */
export function activityTiming(
  keys: string[],
  options: { active: boolean; now: number; arrivals: ToolArrivalTracker },
): ActivityTiming {
  if (keys.length === 0) {
    return NO_TIMING;
  }
  let first = Number.POSITIVE_INFINITY;
  let last = Number.NEGATIVE_INFINITY;
  for (const key of keys) {
    const arrival = options.arrivals.read(key);
    if (!arrival) {
      return NO_TIMING;
    }
    first = Math.min(first, arrival.first);
    last = Math.max(last, arrival.last);
  }
  if (options.active) {
    const liveMs = options.now - first;
    return liveMs >= DURATION_BADGE_MIN_MS ? { liveMs, frozenMs: null } : NO_TIMING;
  }
  const frozenMs = last - first;
  return frozenMs >= DURATION_BADGE_MIN_MS ? { liveMs: null, frozenMs } : NO_TIMING;
}

/** Whole seconds for the elapsed label; a partial second does not read as one. */
export function elapsedSeconds(ms: number): number {
  return Math.max(0, Math.floor(ms / 1000));
}

/**
 * The runtime-published duration, for a badge. Absent or sub-threshold values
 * render nothing rather than a zero.
 */
export function durationBadgeLabel(durationMs: number | null | undefined): string | null {
  if (
    typeof durationMs !== "number" ||
    !Number.isFinite(durationMs) ||
    durationMs < DURATION_BADGE_MIN_MS
  ) {
    return null;
  }
  return `${(durationMs / 1000).toFixed(1)}s`;
}

/** The exact published value, for the expanded card. */
export function exactDurationLabel(durationMs: number): string {
  return `${Math.round(durationMs)} ms`;
}

/**
 * Record arrival times for a dispatched action.
 *
 * Only `stream_event` carries events that reached this client live. `hydrate`
 * and `job_state_synced` are replays: they are ignored, so an activity built
 * from them has no start and therefore no duration.
 */
export function recordArrivalsForAction(
  action: WorkbenchAction,
  at: number,
  tracker: ToolArrivalTracker = toolArrivals,
): void {
  switch (action.type) {
    case "reset":
    case "hydrate":
      tracker.clear();
      return;
    case "stream_event": {
      const key = arrivalKeyForStreamEvent(action.event);
      if (key) {
        tracker.record(key, at);
      }
      return;
    }
    default:
      return;
  }
}

export function arrivalKeyForStreamEvent(event: StreamEvent): string | null {
  switch (event.type) {
    case "tool_call_started":
    case "tool_call_approval_needed":
    case "tool_call_completed":
    case "tool_call_failed":
      return toolArrivalKey(event.call_id);
    case "input_needed":
      return inputArrivalKey(event.input_id);
    default:
      return null;
  }
}
