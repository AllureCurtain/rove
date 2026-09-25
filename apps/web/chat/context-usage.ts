/**
 * Context window occupancy for the composer's usage ring.
 *
 * The window size is optional on the model descriptor, so the ring is only
 * drawn when it is known. While a reply is still streaming its usage is
 * absent; the previous turn's usage stays up instead of flashing to zero.
 */

export const CONTEXT_WARN_REMAINING = 0.25;
export const CONTEXT_DANGER_REMAINING = 0.1;

export interface ContextUsageInput {
  promptTokens: number;
  completionTokens: number;
  cachedTokens?: number;
  /** Model context window. Null when the descriptor does not publish one. */
  window: number | null;
}

export interface ContextUsage {
  used: number;
  window: number | null;
  cached: number;
  /** Share of the window used, or null when the window is unknown. */
  ratio: number | null;
  tone: "neutral" | "warn" | "danger";
  /** Cache hit rate over cached plus uncached input, or null when undefined. */
  cacheRatio: number | null;
}

export function contextUsage(input: ContextUsageInput): ContextUsage {
  const used = Math.max(0, input.promptTokens) + Math.max(0, input.completionTokens);
  const cached = Math.max(0, input.cachedTokens ?? 0);
  const window = input.window && input.window > 0 ? input.window : null;
  const ratio = window === null ? null : Math.min(1, used / window);
  const cacheDenominator = Math.max(0, input.promptTokens) + cached;
  return {
    used,
    window,
    cached,
    ratio,
    tone: toneFor(ratio),
    cacheRatio: cacheDenominator === 0 ? null : cached / cacheDenominator,
  };
}

function toneFor(ratio: number | null): ContextUsage["tone"] {
  if (ratio === null) {
    return "neutral";
  }
  const remaining = 1 - ratio;
  if (remaining <= CONTEXT_DANGER_REMAINING) {
    return "danger";
  }
  if (remaining <= CONTEXT_WARN_REMAINING) {
    return "warn";
  }
  return "neutral";
}

/**
 * Usage to display while a turn is in flight: the latest message that
 * actually carries usage, so a streaming tail without usage does not zero the
 * ring.
 */
export function latestUsage<T>(entries: readonly T[], hasUsage: (entry: T) => boolean): T | null {
  for (let index = entries.length - 1; index >= 0; index -= 1) {
    const entry = entries[index]!;
    if (hasUsage(entry)) {
      return entry;
    }
  }
  return null;
}
