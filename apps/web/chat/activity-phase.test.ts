import type { TranscriptRunGroup } from "../lib/rove-state";
import { describe, expect, it } from "vitest";

import {
  activityEventForStreamEvent,
  activityPhaseKey,
  activityPhaseForTranscript,
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

  it("enters waiting-for-subagents on a wrapped child event and counts distinct parents", () => {
    const activity = activityEventForStreamEvent({
      type: "subagent_event",
      parent_call_id: "call-task-1",
      event: { type: "llm_chunk", delta: "hi" },
    });
    expect(activity).toEqual({
      type: "subagent_event",
      parentCallId: "call-task-1",
    });

    const waiting = reduceActivityPhase(IDLE_PHASE, activity!);
    expect(waiting).toEqual({
      kind: "waiting-for-subagents",
      count: 1,
      callIds: ["call-task-1"],
    });

    // A second delegation under a different parent call raises the count.
    const second = reduceActivityPhase(waiting, {
      type: "subagent_event",
      parentCallId: "call-task-2",
    });
    expect(second).toEqual({
      kind: "waiting-for-subagents",
      count: 2,
      callIds: ["call-task-1", "call-task-2"],
    });

    // More child traffic under an already-counted call keeps the same phase
    // and the same announcement key.
    const steady = reduceActivityPhase(second, {
      type: "subagent_event",
      parentCallId: "call-task-1",
    });
    expect(steady).toBe(second);
    expect(activityPhaseKey(steady)).toBe("waiting-for-subagents:2");
  });

  it("leaves the subagent wait when the parent turn produces its own output", () => {
    const waiting = reduceActivityPhase(IDLE_PHASE, {
      type: "subagent_event",
      parentCallId: "call-task-1",
    });
    expect(reduceActivityPhase(waiting, { type: "llm_chunk" })).toBe(IDLE_PHASE);
    expect(reduceActivityPhase(waiting, { type: "run_completed" })).toBe(IDLE_PHASE);
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

describe("durable activity precedence", () => {
  const waiting: TranscriptRunGroup = {
    id: "run:active", runId: "active", runOrdinal: 2, inherited: false, sourceSessionId: null,
    items: [{
      kind: "tool",
      entry: { id: "call", kind: "tool", entityId: "call", runId: "active", runOrdinal: 2, eventSeq: 1, inherited: false, sourceSessionId: null },
      tool: { id: "call", name: "write_file", status: "waiting", details: "",
        pendingApproval: { call_id: "call", name: "write_file", args: {}, reason: "approval" } },
    }],
  };

  it("keeps restored approvals ahead of execution or child activity", () => {
    expect(activityPhaseForTranscript({ kind: "tool", name: "write_file" }, [waiting], true))
      .toEqual({ kind: "waiting-approval", name: "write_file" });
    expect(activityPhaseForTranscript(IDLE_PHASE, [waiting], true).kind).toBe("waiting-approval");
  });

  it("clears terminal waits and does not resurrect inherited or earlier requests", () => {
    expect(activityPhaseForTranscript({ kind: "tool", name: "write_file" }, [waiting], false)).toBe(IDLE_PHASE);
    expect(activityPhaseForTranscript(IDLE_PHASE, [{ ...waiting, inherited: true }], true)).toBe(IDLE_PHASE);
    expect(activityPhaseForTranscript(IDLE_PHASE, [waiting, { ...waiting, items: [] }], true)).toBe(IDLE_PHASE);
  });

  it("announces a pending input and leaves a submitted input alone", () => {
    const input: TranscriptRunGroup = { ...waiting, items: [{ kind: "input", entry: { id: "input", kind: "input", entityId: "input", runId: "active", runOrdinal: 2, eventSeq: 1, inherited: false, sourceSessionId: null }, input: { id: "input", timelineId: "input", prompt: "Branch?", status: "waiting" } }] };
    expect(activityPhaseForTranscript({ kind: "tool", name: "request_input" }, [input], true).kind).toBe("waiting-input");
    if (input.items[0].kind === "input") input.items[0].input.status = "submitted";
    expect(activityPhaseForTranscript(IDLE_PHASE, [input], true)).toBe(IDLE_PHASE);
  });
});
