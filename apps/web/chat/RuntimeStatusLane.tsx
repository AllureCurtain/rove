"use client";

import { useEffect, useRef, useState } from "react";

import { useCopy } from "../copy/CopyProvider";
import type { ActivityPhase } from "./activity-phase";

/**
 * The transcript tail's single owner of "what is it doing right now".
 *
 * The lane reserves its height for the whole running turn so indicators mount
 * and clear without shifting rows the reader is on, and it renders exactly one
 * indicator at a time. The phase tints the row: waits on the model read as
 * neutral information, retries and degradation warn. When the turn ends the
 * lane itself unmounts — idle owns no space.
 */
export function RuntimeStatusLane({
  busy,
  phase,
}: {
  busy: boolean;
  phase: ActivityPhase;
}) {
  const { t } = useCopy();
  const active = busy || phase.kind !== "idle";
  // Elapsed is measured from the moment the lane went live, not from event
  // payloads: the reader cares how long they have been watching this turn,
  // and a restarted lane is a new wait.
  const startedAtRef = useRef<number | null>(null);
  const [elapsedSeconds, setElapsedSeconds] = useState(0);
  useEffect(() => {
    if (!active) {
      startedAtRef.current = null;
      setElapsedSeconds(0);
      return;
    }
    startedAtRef.current = performance.now();
    setElapsedSeconds(0);
    const timer = window.setInterval(
      () =>
        setElapsedSeconds(
          Math.round((performance.now() - (startedAtRef.current ?? 0)) / 1_000),
        ),
      1_000,
    );
    return () => window.clearInterval(timer);
  }, [active]);

  if (!active) {
    return null;
  }
  const label = activityPhaseLabel(phase, t) ?? t("chat.working");
  return (
    <div className="transcript-runtime-status" role="status" aria-live="polite">
      <span
        className="runtime-status-indicator"
        data-phase={phaseTone(phase)}
      >
        <span className="runtime-status-indicator__dots" aria-hidden="true">
          <i />
          <i />
          <i />
        </span>
        <span className="runtime-status-indicator__label">{label}</span>
        <span className="runtime-status-indicator__elapsed">
          {t("chat.statusElapsed", { n: elapsedSeconds })}
        </span>
      </span>
    </div>
  );
}

/** The tint lane phase groups collapse onto: info / warning / working. */
function phaseTone(phase: ActivityPhase): string {
  switch (phase.kind) {
    case "degraded":
      return "warning";
    case "planning":
      return "accent";
    case "tool":
      return "working";
    case "idle":
      return "working";
    default:
      return "info";
  }
}

function activityPhaseLabel(
  phase: ActivityPhase,
  t: (path: string, params?: Record<string, string | number>) => string,
): string | null {
  switch (phase.kind) {
    case "idle":
      return null;
    case "sending":
      return t("chat.phaseSending");
    case "waiting-model":
      return t("chat.phaseWaitingModel");
    case "compacting":
      return phase.degraded
        ? t("chat.phaseCompactingDegraded")
        : t("chat.phaseCompacting");
    case "tool":
      return t("chat.phaseTool", { name: phase.name });
    case "planning":
      return t("chat.phasePlanning");
    case "finalizing":
      return t("chat.phaseFinalizing");
    case "degraded":
      return t("chat.phaseDegraded", { summary: phase.summary });
  }
}
