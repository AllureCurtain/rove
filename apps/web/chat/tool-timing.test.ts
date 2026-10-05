import { describe, expect, it } from "vitest";

import type { StreamEvent } from "../lib/rove-types";
import type { WorkbenchAction } from "../lib/rove-state";
import {
  DURATION_BADGE_MIN_MS,
  activityTiming,
  arrivalKeyForStreamEvent,
  createToolArrivalTracker,
  durationBadgeLabel,
  elapsedSeconds,
  exactDurationLabel,
  inputArrivalKey,
  recordArrivalsForAction,
  toolArrivalKey,
} from "./tool-timing";

function streamEvent(event: StreamEvent): WorkbenchAction {
  return { type: "stream_event", event, seq: 1 };
}

describe("tool arrival keys", () => {
  it("names tools and inputs by their own identity", () => {
    expect(toolArrivalKey("call-1")).toBe("tool:call-1");
    expect(inputArrivalKey("input-1")).toBe("input:input-1");
  });

  it("covers every event that moves a tool or input", () => {
    expect(
      arrivalKeyForStreamEvent({
        type: "tool_call_started",
        call_id: "call-1",
        name: "read_file",
        args: {},
      }),
    ).toBe("tool:call-1");
    expect(
      arrivalKeyForStreamEvent({
        type: "tool_call_approval_needed",
        call_id: "call-2",
        name: "write_file",
        args: {},
        reason: "destructive",
      }),
    ).toBe("tool:call-2");
    expect(
      arrivalKeyForStreamEvent({
        type: "tool_call_completed",
        call_id: "call-3",
        result: { call_id: "call-3", output: "ok" },
      }),
    ).toBe("tool:call-3");
    expect(
      arrivalKeyForStreamEvent({
        type: "tool_call_failed",
        call_id: "call-4",
        error: { code: "tool_failed", message: "boom" },
      }),
    ).toBe("tool:call-4");
    expect(
      arrivalKeyForStreamEvent({ type: "input_needed", input_id: "input-1", prompt: "?" }),
    ).toBe("input:input-1");
    expect(
      arrivalKeyForStreamEvent({
        type: "llm_chunk",
        delta: "text",
      }),
    ).toBeNull();
  });
});

describe("arrival recording", () => {
  it("keeps the first and the latest live arrival", () => {
    const tracker = createToolArrivalTracker();
    tracker.record("tool:call-1", 100);
    tracker.record("tool:call-1", 900);
    expect(tracker.read("tool:call-1")).toEqual({ first: 100, last: 900 });
    expect(tracker.size()).toBe(1);
  });

  it("records live stream events and ignores snapshot replays", () => {
    const tracker = createToolArrivalTracker();
    recordArrivalsForAction(
      streamEvent({
        type: "tool_call_started",
        call_id: "call-1",
        name: "read_file",
        args: {},
      }),
      500,
      tracker,
    );
    recordArrivalsForAction(
      {
        type: "job_state_synced",
        state: {
          job_id: "job-1",
          run_id: "run-1",
          status: "running",
          event_count: 9,
          events: [
            {
              seq: 9,
              event: { type: "tool_call_started", call_id: "call-9", name: "grep", args: {} },
            },
          ],
        },
      } as WorkbenchAction,
      900,
      tracker,
    );

    expect(tracker.read("tool:call-1")).toEqual({ first: 500, last: 500 });
    expect(tracker.read("tool:call-9")).toBeUndefined();
  });

  it("clears the history when the transcript is replaced", () => {
    const tracker = createToolArrivalTracker();
    tracker.record("tool:call-1", 100);
    recordArrivalsForAction({ type: "reset" }, 200, tracker);
    expect(tracker.size()).toBe(0);

    tracker.record("tool:call-2", 300);
    recordArrivalsForAction({ type: "hydrate", state: {} as never }, 400, tracker);
    expect(tracker.size()).toBe(0);
  });

  it("leaves unrelated actions alone", () => {
    const tracker = createToolArrivalTracker();
    tracker.record("tool:call-1", 100);
    recordArrivalsForAction({ type: "set_busy", busy: true }, 200, tracker);
    recordArrivalsForAction({ type: "job_created", jobId: "job-1", runId: "run-1" }, 300, tracker);
    expect(tracker.size()).toBe(1);
  });
});

describe("activity timing", () => {
  it("ticks while the activity is still in flight", () => {
    const tracker = createToolArrivalTracker();
    tracker.record("tool:call-1", 1_000);
    expect(activityTiming(["tool:call-1"], { active: true, now: 2_400, arrivals: tracker })).toEqual({
      liveMs: 1_400,
      frozenMs: null,
    });
  });

  it("stays hidden below the display threshold", () => {
    const tracker = createToolArrivalTracker();
    tracker.record("tool:call-1", 1_000);
    expect(activityTiming(["tool:call-1"], { active: true, now: 1_400, arrivals: tracker })).toEqual({
      liveMs: null,
      frozenMs: null,
    });
  });

  it("freezes on the last arrival once the activity settles", () => {
    const tracker = createToolArrivalTracker();
    tracker.record("tool:call-1", 1_000);
    tracker.record("tool:call-1", 4_200);
    expect(activityTiming(["tool:call-1"], { active: false, now: 9_000, arrivals: tracker })).toEqual({
      liveMs: null,
      frozenMs: 3_200,
    });
  });

  it("spans every item of a multi-call activity", () => {
    const tracker = createToolArrivalTracker();
    tracker.record("tool:call-1", 1_000);
    tracker.record("tool:call-1", 1_300);
    tracker.record("tool:call-2", 2_000);
    tracker.record("tool:call-2", 3_000);
    expect(
      activityTiming(["tool:call-1", "tool:call-2"], {
        active: false,
        now: 5_000,
        arrivals: tracker,
      }),
    ).toEqual({ liveMs: null, frozenMs: 2_000 });
  });

  it("refuses to invent a duration for an item it never saw arrive", () => {
    const tracker = createToolArrivalTracker();
    tracker.record("tool:call-1", 1_000);
    // call-2 came from a restore or a replay batch; the activity has no start.
    expect(
      activityTiming(["tool:call-1", "tool:call-2"], {
        active: true,
        now: 9_000,
        arrivals: tracker,
      }),
    ).toEqual({ liveMs: null, frozenMs: null });
  });

  it("reports nothing without identity", () => {
    const tracker = createToolArrivalTracker();
    expect(activityTiming([], { active: true, now: 9_000, arrivals: tracker })).toEqual({
      liveMs: null,
      frozenMs: null,
    });
  });

  it("refuses a span that runs backwards instead of reporting zero", () => {
    const tracker = createToolArrivalTracker();
    tracker.record("tool:call-1", 5_000);
    tracker.record("tool:call-1", 2_000);
    // A backwards span is not a duration; showing 0s would be a claim.
    const timing = activityTiming(["tool:call-1"], {
      active: false,
      now: 9_000,
      arrivals: tracker,
    });
    expect(timing).toEqual({ liveMs: null, frozenMs: null });
  });
});

describe("duration labels", () => {
  it("badges only at or above the threshold", () => {
    expect(durationBadgeLabel(999)).toBeNull();
    expect(durationBadgeLabel(0)).toBeNull();
    expect(durationBadgeLabel(undefined)).toBeNull();
    expect(durationBadgeLabel(Number.NaN)).toBeNull();
    expect(durationBadgeLabel(DURATION_BADGE_MIN_MS)).toBe("1.0s");
    expect(durationBadgeLabel(2_450)).toBe("2.5s");
    expect(durationBadgeLabel(60_000)).toBe("60.0s");
  });

  it("keeps the exact value for the expanded card", () => {
    expect(exactDurationLabel(2_450)).toBe("2450 ms");
    expect(exactDurationLabel(12)).toBe("12 ms");
  });

  it("reads elapsed whole seconds", () => {
    expect(elapsedSeconds(1_000)).toBe(1);
    expect(elapsedSeconds(1_999)).toBe(1);
    expect(elapsedSeconds(2_000)).toBe(2);
    expect(elapsedSeconds(-5)).toBe(0);
  });
});
