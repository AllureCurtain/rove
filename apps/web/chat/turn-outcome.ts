import type { TranscriptRunGroup } from "../lib/rove-state";
import {
  activityTiming,
  inputArrivalKey,
  toolArrivalKey,
  toolArrivals,
} from "./tool-timing";

export interface TurnOutcome {
  /** Tool calls the run made — the receipt's "steps". */
  actions: number;
  /** Summed assistant usage for the run; null when no message reported any. */
  tokens: number | null;
  /**
   * Wall-clock span of the run's work as this client observed it. Null when the
   * run was restored rather than watched live — an invented number is worse
   * than none.
   */
  durationMs: number | null;
}

/**
 * The end-of-turn receipt ("9 steps · 42.6s · 18.2k tokens"). Only facts the
 * run group itself carries are summed; a restored run without arrival timing
 * simply shows fewer segments.
 */
export function runGroupOutcome(group: TranscriptRunGroup): TurnOutcome | null {
  let actions = 0;
  let tokens = 0;
  let anyTokens = false;
  const arrivalKeys: string[] = [];
  for (const item of group.items) {
    if (item.kind === "tool") {
      actions += 1;
      arrivalKeys.push(toolArrivalKey(item.tool.id));
    } else if (item.kind === "input") {
      arrivalKeys.push(inputArrivalKey(item.input.id));
    } else if (
      item.kind === "message" &&
      item.message.role === "assistant" &&
      item.message.usage
    ) {
      tokens += item.message.usage.total_tokens;
      anyTokens = true;
    }
  }
  const timing = activityTiming(arrivalKeys, {
    active: false,
    now: 0,
    arrivals: toolArrivals,
  });
  const outcome = {
    actions,
    // A zero carries no information — "0 tokens" reads as noise under every
    // finished turn, so the segment only appears once it can say something.
    tokens: anyTokens && tokens > 0 ? tokens : null,
    durationMs: timing.frozenMs,
  };
  return outcome.actions > 0 || outcome.tokens !== null ? outcome : null;
}

/** Compact token count for the receipt: 842 → "842", 18_243 → "18.2k". */
export function formatReceiptTokens(tokens: number): string {
  if (tokens >= 10_000) {
    const rounded = Math.round(tokens / 100) / 10;
    return `${rounded}k`;
  }
  if (tokens >= 1_000) {
    return `${Math.round(tokens / 1_000)}k`;
  }
  return String(tokens);
}

/** Receipt duration: seconds with one decimal under a minute, m:ss beyond. */
export function formatReceiptDuration(ms: number): string {
  const seconds = ms / 1_000;
  if (seconds < 60) {
    return `${Math.round(seconds * 10) / 10}s`;
  }
  const whole = Math.round(seconds);
  return `${Math.floor(whole / 60)}:${String(whole % 60).padStart(2, "0")}`;
}
