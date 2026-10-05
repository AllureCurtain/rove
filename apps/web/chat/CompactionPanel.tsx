"use client";

import { useCallback, useEffect, useRef, useState } from "react";

import { useCopy } from "../copy/CopyProvider";
import type { ProductSessionCompaction } from "../product/product-api-types";
import { ProductApiError } from "../product/product-client";
import {
  compactionAvailability,
  compactionOutcome,
  type CompactionOutcome,
} from "./session-compaction";

export interface CompactionPanelProps {
  sessionId: string | null;
  sessionStatus: string | null;
  busy: boolean;
  client: {
    compactSession: (sessionId: string) => Promise<ProductSessionCompaction>;
  };
}

/**
 * Manual context compaction (R9), the HTTP equivalent of the CLI's `/compact`.
 *
 * The endpoint takes the same per-session claim a turn takes, so it is offered
 * only at idle and a refusal is never retried: the call is not queued behind the
 * running turn, it is simply refused. Everything the panel shows is a fact the
 * API sent. The response describes *this call* (`triggered`) next to the state
 * the session now holds (`mode`, `degraded`, `circuit_open`, the bounded
 * excerpt), and the two answer different questions, so "nothing to compact",
 * "already compacted" and "the breaker refuses the automatic path" stay
 * distinguishable. Only the typed `failure_code` classifies a failed summary;
 * the provider's own message is never in the response and never rendered.
 */
export function CompactionPanel({
  sessionId,
  sessionStatus,
  busy,
  client,
}: CompactionPanelProps) {
  const { t } = useCopy();
  const [running, setRunning] = useState(false);
  const [outcome, setOutcome] = useState<CompactionOutcome | null>(null);
  const [error, setError] = useState<{ key: string; params?: Record<string, string> } | null>(
    null,
  );
  // A second call must not race the first: the endpoint is not concurrency-safe
  // (only one caller can hold the claim), so the second press is a no-op.
  const runningRef = useRef(false);

  // The result belongs to the session that produced it. A session switch clears
  // it rather than showing one session's compaction under another's name.
  useEffect(() => {
    runningRef.current = false;
    setRunning(false);
    setOutcome(null);
    setError(null);
  }, [sessionId]);

  const availability = compactionAvailability({ sessionId, sessionStatus, busy });

  const compact = useCallback(async () => {
    if (!sessionId || !availability.available || runningRef.current) {
      return;
    }
    runningRef.current = true;
    setRunning(true);
    setError(null);
    try {
      const compaction = await client.compactSession(sessionId);
      setOutcome(compactionOutcome(compaction));
    } catch (failure) {
      if (failure instanceof ProductApiError && failure.code === "product_session_active") {
        // The session is still running. The endpoint did not queue this call, so
        // retrying it would only be refused again; say so and stop.
        setError({ key: "chat.compactActive" });
      } else {
        setError({
          key: "chat.compactFailed",
          params: {
            error:
              failure instanceof ProductApiError
                ? failure.code
                : t("search.unknownError"),
          },
        });
      }
    } finally {
      runningRef.current = false;
      setRunning(false);
    }
  }, [availability.available, client, sessionId, t]);

  return (
    <section className="compaction-panel" aria-label={t("chat.compactAction")}>
      <button
        type="button"
        className="secondary"
        disabled={!availability.available || running}
        title={
          availability.reasonCopyKey ? t(availability.reasonCopyKey) : t("chat.compactAction")
        }
        onClick={() => void compact()}
      >
        {running ? t("chat.compactBusy") : t("chat.compactAction")}
      </button>
      {!availability.available && availability.reasonCopyKey ? (
        <p className="compaction-panel__reason">{t(availability.reasonCopyKey)}</p>
      ) : null}
      {error ? (
        <p className="compaction-panel__error" role="alert">
          {t(error.key, error.params)}
        </p>
      ) : null}
      {outcome ? <CompactionResult outcome={outcome} /> : null}
    </section>
  );
}

function CompactionResult({ outcome }: { outcome: CompactionOutcome }) {
  const { t } = useCopy();
  return (
    <div className="compaction-panel__result" role="status">
      {outcome.status === "no-run" ? <p>{t("chat.compactNoRun")}</p> : null}
      {outcome.status === "no-op" ? (
        <p>
          {outcome.circuitOpen
            ? t("chat.compactNoopCircuit", { count: outcome.consecutiveFailures })
            : t("chat.compactNoop")}
        </p>
      ) : null}
      {outcome.status === "compacted" ? <p>{t("chat.compactDone")}</p> : null}

      <p>
        {t("chat.compactMode", {
          // An unknown mode is shown as the API sent it; guessing a translation
          // would report a fact the API did not send.
          mode: outcome.mode.copyKey
            ? t(outcome.mode.copyKey)
            : outcome.mode.raw,
        })}
      </p>

      {outcome.degraded ? (
        <p className="compaction-panel__degraded">
          {outcome.failureCode
            ? t("chat.compactDegraded", { code: outcome.failureCode })
            : t("chat.compactDegradedNoCode")}
        </p>
      ) : null}

      {/* `circuit_open` describes persisted session history, not this request:
          a manual run still spends one bounded probe even while the automatic
          path is refused. The refusal window is only stated when it was sent. */}
      {outcome.circuitOpen ? (
        <p>
          {t("chat.compactCircuit", { count: outcome.consecutiveFailures })}
          {outcome.nextAttemptAfter
            ? ` ${t("chat.compactCircuitUntil", { until: outcome.nextAttemptAfter })}`
            : ""}
        </p>
      ) : null}

      {outcome.summary !== null ? (
        <div>
          <p className="compaction-panel__summary-label">{t("chat.compactSummaryLabel")}</p>
          <pre tabIndex={0}>{outcome.summary}</pre>
          {outcome.summaryTruncated ? (
            // The API bounded the excerpt; presenting it as the whole summary
            // would claim more than was sent.
            <p>{t("chat.compactSummaryTruncated")}</p>
          ) : null}
        </div>
      ) : null}

      <p>{t("chat.compactTokens", { tokens: outcome.tokenEstimate })}</p>
      <p>{t("chat.compactSourceMessages", { count: outcome.sourceMessageCount })}</p>
    </div>
  );
}
