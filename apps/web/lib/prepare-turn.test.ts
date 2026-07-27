import { describe, expect, it } from "vitest";

import {
  createWorkbenchState,
  workbenchReducer,
} from "../lib/rove-state";

describe("prepare_turn", () => {
  it("keeps transcript messages while clearing run-scoped state", () => {
    let state = createWorkbenchState();
    state = workbenchReducer(state, {
      type: "append_user_message",
      content: "first",
    });
    state = workbenchReducer(state, {
      type: "job_created",
      jobId: "job-1",
      runId: "run-1",
      resumedFromRunId: null,
    });
    state = workbenchReducer(state, {
      type: "stream_event",
      seq: 1,
      event: {
        type: "llm_chunk",
        delta: "hello",
      },
    });

    const prepared = workbenchReducer(state, { type: "prepare_turn" });
    expect(prepared.messages).toHaveLength(2);
    expect(prepared.activeJobId).toBeNull();
    expect(prepared.tools).toEqual([]);
    expect(prepared.eventCount).toBe(0);
    expect(prepared.seenEventSeqs).toEqual([]);
  });

  it("can retain terminal product tool history without carrying pending work", () => {
    const state = {
      ...createWorkbenchState(),
      tools: [
        {
          id: "call-done",
          name: "read_file",
          status: "done" as const,
          details: "complete",
        },
        {
          id: "call-waiting",
          name: "write_file",
          status: "waiting" as const,
          details: "approval required",
        },
      ],
    };

    const prepared = workbenchReducer(state, {
      type: "prepare_turn",
      preserveTools: true,
    });

    expect(prepared.tools).toEqual([]);
    expect(prepared.historicalTools).toEqual([state.tools[0]]);
    expect(prepared.pendingInputs).toEqual([]);
  });
});
