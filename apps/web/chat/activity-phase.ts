/**
 * What the composer tells the reader while a turn produces no text.
 *
 * The runtime reports these as separate events (model status, compaction,
 * degradation). Collapsing them into one spinner hides the difference between
 * "thinking", "compacting" and "something went wrong". Unknown statuses fall
 * back to waiting rather than being shown verbatim.
 */

import type { StreamEvent } from "../lib/rove-types";

export type ActivityPhase =
  | { kind: "idle" }
  | { kind: "sending" }
  | { kind: "waiting-model" }
  | { kind: "compacting"; degraded: boolean }
  | { kind: "tool"; name: string }
  | { kind: "planning" }
  | { kind: "finalizing" }
  | { kind: "degraded"; summary: string };

export type ActivityEvent =
  | { type: "reset" }
  | { type: "send" }
  | { type: "run_started" }
  | { type: "llm_chunk" }
  | { type: "run_completed" }
  | { type: "model_status"; status: string }
  | { type: "prompt_compacted"; degraded: boolean }
  | { type: "tool_call_started"; name: string }
  | { type: "plan_step_started" }
  | { type: "finalization_started" }
  | { type: "execution_degraded"; summary: string };

const KNOWN_MODEL_STATUSES = new Set(["queued", "running", "retrying", "waiting"]);

export const IDLE_PHASE: ActivityPhase = { kind: "idle" };

export function reduceActivityPhase(phase: ActivityPhase, event: ActivityEvent): ActivityPhase {
  switch (event.type) {
    case "reset":
      return IDLE_PHASE;
    case "send":
      return { kind: "sending" };
    case "run_started":
      return { kind: "waiting-model" };
    case "model_status":
      return KNOWN_MODEL_STATUSES.has(event.status) ? { kind: "waiting-model" } : phase;
    case "prompt_compacted":
      return { kind: "compacting", degraded: event.degraded };
    case "tool_call_started":
      return { kind: "tool", name: event.name };
    case "plan_step_started":
      return { kind: "planning" };
    case "finalization_started":
      return { kind: "finalizing" };
    case "execution_degraded":
      return { kind: "degraded", summary: event.summary };
    case "llm_chunk":
    case "run_completed":
      return IDLE_PHASE;
  }
}

/**
 * Map a canonical stream event onto an activity event. Events the phase does
 * not express (traces, artifacts, plan bookkeeping) return null and leave the
 * phase untouched.
 */
export function activityEventForStreamEvent(event: StreamEvent): ActivityEvent | null {
  switch (event.type) {
    case "run_started":
      return { type: "run_started" };
    case "llm_chunk":
      return { type: "llm_chunk" };
    case "run_completed":
      return { type: "run_completed" };
    case "model_status":
      return { type: "model_status", status: event.status };
    case "prompt_compacted":
      return {
        type: "prompt_compacted",
        degraded: Boolean(event.state.degraded) || Boolean(event.state.circuit_open),
      };
    case "tool_call_started":
      return { type: "tool_call_started", name: event.name };
    case "plan_step_started":
      return { type: "plan_step_started" };
    case "finalization_started":
      return { type: "finalization_started" };
    case "execution_degraded":
      return { type: "execution_degraded", summary: event.record.safe_summary };
    default:
      return null;
  }
}

/** Stable text for a phase, so repeat events do not change what is announced. */
export function activityPhaseKey(phase: ActivityPhase): string {
  switch (phase.kind) {
    case "tool":
      return `tool:${phase.name}`;
    case "compacting":
      return `compacting:${phase.degraded}`;
    case "degraded":
      return `degraded:${phase.summary}`;
    default:
      return phase.kind;
  }
}
