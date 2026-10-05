import {
  isKnownProductCompactionMode,
  type ProductSessionCompaction,
} from "../product/product-api-types";

/**
 * What one manual compaction call actually reported, with every fact the API
 * sent kept apart from the ones it did not (R9).
 *
 * The endpoint returns *this call* (`triggered`) next to *the state the session
 * now holds* (`mode`, `degraded`, `circuit_open`, the excerpt), and those answer
 * different questions: "nothing to compact", "already compacted", and "the
 * breaker refuses the automatic path" must not collapse into one sentence. A
 * failure is only ever the typed `failure_code`; the provider's own message is
 * never in the response, so it is never rendered.
 */
export interface CompactionOutcome {
  /** Absent `runtime_run_id` is the server saying there is no run to compact. */
  status: "no-run" | "no-op" | "compacted";
  /** The stored summary is the deterministic fallback, not a model summary. */
  degraded: boolean;
  failureCode: string | null;
  mode: { copyKey: string | null; raw: string };
  circuitOpen: boolean;
  consecutiveFailures: number;
  nextAttemptAfter: string | null;
  summary: string | null;
  summaryTruncated: boolean;
  tokenEstimate: number;
  sourceMessageCount: number;
}

const COMPACTION_MODE_COPY_KEYS: Record<string, string> = {
  none: "chat.compactModeNone",
  deterministic: "chat.compactModeDeterministic",
  model_generated: "chat.compactModeModelGenerated",
  automatic: "chat.compactModeAutomatic",
  degraded: "chat.compactModeDegraded",
  disabled: "chat.compactModeDisabled",
};

/**
 * Translate a mode this build knows; an unknown mode is shown verbatim rather
 * than replaced by a guess, because the API reported it and the UI may not
 * invent a different fact.
 */
export function compactionModeLabel(mode: string): {
  copyKey: string | null;
  raw: string;
} {
  const copyKey = isKnownProductCompactionMode(mode)
    ? (COMPACTION_MODE_COPY_KEYS[mode] ?? null)
    : null;
  return { copyKey, raw: mode };
}

export function compactionOutcome(
  compaction: ProductSessionCompaction,
): CompactionOutcome {
  const status =
    compaction.runtime_run_id === undefined
      ? "no-run"
      : compaction.triggered
        ? "compacted"
        : "no-op";
  return {
    status,
    degraded: compaction.degraded,
    failureCode: compaction.failure_code ?? null,
    mode: compactionModeLabel(compaction.mode),
    circuitOpen: compaction.circuit_open,
    consecutiveFailures: compaction.consecutive_failures,
    nextAttemptAfter: compaction.next_attempt_after ?? null,
    summary: compaction.summary ?? null,
    summaryTruncated: compaction.summary_truncated,
    tokenEstimate: compaction.token_estimate,
    sourceMessageCount: compaction.source_message_count,
  };
}

/**
 * Whether the session may be compacted right now.
 *
 * The endpoint takes the same per-session claim a turn takes, so a call while
 * the session is running is refused with 409 `product_session_active` and is
 * *not* queued behind the turn. The waiting row's compacting phase is a
 * degraded display of an automatic compaction, never a licence to call this
 * endpoint, so the action is offered only at idle.
 */
export function compactionAvailability(input: {
  sessionId: string | null;
  sessionStatus: string | null;
  busy: boolean;
}): { available: boolean; reasonCopyKey: string | null } {
  if (!input.sessionId) {
    return { available: false, reasonCopyKey: "commandPalette.unavailableNoSession" };
  }
  if (input.busy || input.sessionStatus === "running") {
    return { available: false, reasonCopyKey: "chat.compactIdleOnly" };
  }
  return { available: true, reasonCopyKey: null };
}
