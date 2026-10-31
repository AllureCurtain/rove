"use client";

import { Component2Icon, StopIcon } from "@radix-ui/react-icons";
import {
  useEffect,
  useState,
  useSyncExternalStore,
} from "react";

import type { SubagentView, ToolCallView } from "../lib/rove-state";
import { useCopy } from "../copy/CopyProvider";
import {
  activityTiming,
  elapsedSeconds,
  toolArrivalKey,
  toolArrivals,
} from "./tool-timing";

/**
 * A stop request in flight per parent `task` call id. The set is shared by the
 * transcript topology card and the work-panel tab, so a stop pressed on one
 * surface disables the other — a duplicate request can never race the first.
 */
const pendingStops = new Set<string>();
const pendingStopListeners = new Set<() => void>();

function subscribePendingStops(listener: () => void): () => void {
  pendingStopListeners.add(listener);
  return () => pendingStopListeners.delete(listener);
}

function stopPending(parentCallId: string): boolean {
  return pendingStops.has(parentCallId);
}

function markStopPending(parentCallId: string, pending: boolean): void {
  const changed = pending
    ? !pendingStops.has(parentCallId)
    : pendingStops.delete(parentCallId);
  if (!changed) {
    return;
  }
  if (pending) {
    pendingStops.add(parentCallId);
  }
  for (const listener of pendingStopListeners) {
    listener();
  }
}

/**
 * The stop affordance a running delegated child carries. It is the only place
 * a subagent can be interrupted from, and it reports the shared pending set so
 * a stop requested from another surface already reads as "stopping".
 */
export function SubagentStopButton({
  parentCallId,
  running,
  onStop,
}: {
  parentCallId: string;
  running: boolean;
  onStop?: (parentCallId: string) => void | Promise<unknown>;
}) {
  const { t } = useCopy();
  const pending = useSyncExternalStore(
    subscribePendingStops,
    () => stopPending(parentCallId),
    () => stopPending(parentCallId),
  );
  // A settle that lands while the request is in flight clears the row's own
  // pending flag: the child is gone, so "stopping" would be a stale claim.
  useEffect(() => {
    if (!running && pending) {
      markStopPending(parentCallId, false);
    }
  }, [running, pending, parentCallId]);
  if (!running || !onStop) {
    return null;
  }
  const label = pending ? t("chat.subagentStopping") : t("chat.subagentStop");
  return (
    <button
      type="button"
      className="subagent-stop"
      aria-label={label}
      title={label}
      disabled={pending}
      onClick={(event) => {
        event.stopPropagation();
        if (stopPending(parentCallId)) {
          return;
        }
        markStopPending(parentCallId, true);
        // A request that settles while the child is still running — refused or
        // failed — hands the row back its affordance; a real settle clears it
        // for every surface through the running-flag effect above.
        Promise.resolve()
          .then(() => onStop(parentCallId))
          .catch(() => undefined)
          .finally(() => markStopPending(parentCallId, false));
      }}
    >
      <StopIcon aria-hidden="true" />
      <span>{label}</span>
    </button>
  );
}

/**
 * The name a topology row leads with: the `agent` argument when the parent
 * asked for one, else a trimmed first line of the delegated prompt, else the
 * bare call id — the row always names something real.
 */
export function subagentRowLabel(tool: ToolCallView): string {
  const args = tool.args;
  if (args && typeof args === "object" && !Array.isArray(args)) {
    const agent = (args as { agent?: unknown }).agent;
    if (typeof agent === "string" && agent.trim()) {
      return agent.trim();
    }
    const prompt = (args as { prompt?: unknown }).prompt;
    if (typeof prompt === "string" && prompt.trim()) {
      const firstLine = prompt.trim().split("\n", 1)[0]!;
      return firstLine.length > 80 ? `${firstLine.slice(0, 80)}…` : firstLine;
    }
  }
  return tool.id;
}

/** The one-word state for a delegated row, in the tool-row vocabulary. */
export function subagentStatusLabel(
  status: SubagentView["status"],
  t: (path: string) => string,
): string {
  switch (status) {
    case "running":
      return t("chat.toolStateRunning");
    case "error":
      return t("chat.toolStateError");
    case "cancelled":
      return t("chat.subagentStateCancelled");
    default:
      return t("chat.toolStateDone");
  }
}

/**
 * The one-level fan-out a `task` call drives (design §5.4): the parent is the
 * coordinator, and each delegated child is one truthful row — status, name,
 * model, steps, elapsed — with a stop while it is still running.
 *
 * The runtime has no delegate-to-delegate edges, so the card deliberately
 * stops at coordinator → children instead of implying a deeper graph.
 */
export function SubagentTopology({
  tool,
  subagents,
  onStopSubagent,
}: {
  tool: ToolCallView;
  subagents: SubagentView[];
  onStopSubagent?: (parentCallId: string) => void | Promise<unknown>;
}) {
  const { t } = useCopy();
  const anyRunning = subagents.some((subagent) => subagent.status === "running");
  const [elapsedNow, setElapsedNow] = useState<number | null>(null);
  useEffect(() => {
    if (!anyRunning) {
      setElapsedNow(null);
      return;
    }
    setElapsedNow(performance.now());
    const timer = window.setInterval(
      () => setElapsedNow(performance.now()),
      1_000,
    );
    return () => window.clearInterval(timer);
  }, [anyRunning]);
  // The parent `task` call's own live arrival is the honest clock a child runs
  // on: the delegation started when the call did, and no earlier.
  const timing = activityTiming([toolArrivalKey(tool.id)], {
    active: anyRunning,
    now: elapsedNow ?? 0,
    arrivals: toolArrivals,
  });
  const elapsedLabel =
    timing.liveMs !== null
      ? t("chat.activityElapsed", { n: elapsedSeconds(timing.liveMs) })
      : null;

  return (
    <section className="subagent-topology" data-testid="subagent-topology">
      <div className="subagent-topology__root">
        <Component2Icon aria-hidden="true" />
        <strong>
          {subagents.length === 1
            ? t("chat.subagentOrchestratingOne")
            : t("chat.subagentOrchestrating", { n: subagents.length })}
        </strong>
        {elapsedLabel ? (
          <span className="subagent-topology__elapsed">{elapsedLabel}</span>
        ) : null}
      </div>
      <ul
        className="subagent-topology__agents"
        role="list"
        aria-label={t("chat.subagentTopologyLabel")}
      >
        {subagents.map((subagent) => (
          <li
            className="subagent-topology__agent"
            data-status={subagent.status}
            key={subagent.parentCallId}
          >
            <span
              className="subagent-topology__dot"
              data-status={subagent.status}
              aria-hidden="true"
            />
            <span className="subagent-topology__name">
              {subagentRowLabel(tool)}
            </span>
            {subagent.model ? (
              <span className="subagent-topology__model">{subagent.model}</span>
            ) : null}
            <span className="subagent-topology__steps">
              {t("chat.subagentSteps", { n: subagent.eventCount })}
            </span>
            <span className="subagent-topology__state" data-status={subagent.status}>
              {subagentStatusLabel(subagent.status, t)}
            </span>
            <SubagentStopButton
              parentCallId={subagent.parentCallId}
              running={subagent.status === "running"}
              onStop={onStopSubagent}
            />
          </li>
        ))}
      </ul>
    </section>
  );
}
