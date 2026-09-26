import { describe, expect, it } from "vitest";

import {
  activityEventForStreamEvent,
  activityPhaseKey,
  IDLE_PHASE,
  reduceActivityPhase,
} from "./activity-phase";

describe("reduceActivityPhase", () => {
  it("moves from sending to waiting once the run starts", () => {
    const sending = reduceActivityPhase(IDLE_PHASE, { type: "send" });
    expect(sending.kind).toBe("sending");
    const waiting = reduceActivityPhase(sending, { type: "run_started" });
    expect(waiting.kind).toBe("waiting-model");
  });

  it("maps a known model status to waiting and ignores an unknown one", () => {
    const known = reduceActivityPhase(IDLE_PHASE, { type: "model_status", status: "retrying" });
    expect(known.kind).toBe("waiting-model");
    const unknown = reduceActivityPhase(IDLE_PHASE, { type: "model_status", status: "frobnicating" });
    expect(unknown).toBe(IDLE_PHASE);
  });

  it("keeps announcing the model phase across a silent-turn recovery", () => {
    const activity = activityEventForStreamEvent({
      type: "model_status",
      status: "recovering_silent_turn",
      message: "Your previous turn produced no visible response.",
    });
    expect(activity).toEqual({
      type: "model_status",
      status: "recovering_silent_turn",
    });
    expect(reduceActivityPhase(IDLE_PHASE, activity!).kind).toBe("waiting-model");
  });

  it("reports compaction with its degraded flag", () => {
    const phase = reduceActivityPhase(IDLE_PHASE, { type: "prompt_compacted", degraded: true });
    expect(phase).toEqual({ kind: "compacting", degraded: true });
  });

  it("returns to idle when text arrives or the run ends", () => {
    const working = reduceActivityPhase(IDLE_PHASE, { type: "tool_call_started", name: "read" });
    expect(reduceActivityPhase(working, { type: "llm_chunk" })).toBe(IDLE_PHASE);
    expect(reduceActivityPhase(working, { type: "run_completed" })).toBe(IDLE_PHASE);
  });

  it("keeps a stable key for repeated events of the same phase", () => {
    const first = reduceActivityPhase(IDLE_PHASE, { type: "tool_call_started", name: "read" });
    const second = reduceActivityPhase(first, { type: "tool_call_started", name: "read" });
    expect(activityPhaseKey(first)).toBe(activityPhaseKey(second));
  });

  it("returns to waiting when the runtime schedules a retry", () => {
    const activity = activityEventForStreamEvent({
      type: "provider_retry",
      attempt: 2,
      max_attempts: 4,
      delay_ms: 2_000,
      reason: "transient:request_failed",
      phase: "model_call",
    });
    expect(activity).toEqual({ type: "model_status", status: "retrying" });
    expect(
      reduceActivityPhase({ kind: "tool", name: "read" }, activity!).kind,
    ).toBe("waiting-model");
  });
});
