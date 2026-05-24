import { describe, expect, it } from "vitest";

import { createWorkbenchState, workbenchReducer } from "./rove-state";

describe("workbenchReducer", () => {
  it("marks an approved waiting tool as running and clears the pending approval", () => {
    const withWaitingTool = workbenchReducer(createWorkbenchState(), {
      type: "stream_event",
      event: {
        type: "tool_call_approval_needed",
        call_id: "call-1",
        name: "fs_write",
        args: { path: "foo.txt" },
        reason: "destructive tool requires explicit approval",
      },
    });

    const approved = workbenchReducer(withWaitingTool, {
      type: "approval_decision",
      callId: "call-1",
      decision: "approve",
    });

    expect(approved.tools[0]).toMatchObject({
      id: "call-1",
      status: "running",
    });
    expect(approved.tools[0].pendingApproval).toBeUndefined();
  });

  it("marks a rejected waiting tool as errored and clears the pending approval", () => {
    const withWaitingTool = workbenchReducer(createWorkbenchState(), {
      type: "stream_event",
      event: {
        type: "tool_call_approval_needed",
        call_id: "call-1",
        name: "fs_write",
        args: { path: "foo.txt" },
        reason: "destructive tool requires explicit approval",
      },
    });

    const rejected = workbenchReducer(withWaitingTool, {
      type: "approval_decision",
      callId: "call-1",
      decision: "reject",
    });

    expect(rejected.tools[0]).toMatchObject({
      id: "call-1",
      status: "error",
      details: "Rejected by user",
    });
    expect(rejected.tools[0].pendingApproval).toBeUndefined();
  });

  it("preserves pending approval details on approval-needed events", () => {
    const state = workbenchReducer(createWorkbenchState(), {
      type: "stream_event",
      event: {
        type: "tool_call_approval_needed",
        call_id: "call-2",
        name: "shell",
        args: { command: "rm -rf /tmp/test" },
        reason: "destructive tool requires explicit approval",
      },
    });

    expect(state.tools[0]).toMatchObject({
      id: "call-2",
      status: "waiting",
      pendingApproval: {
        call_id: "call-2",
        name: "shell",
        reason: "destructive tool requires explicit approval",
      },
    });
  });

  it("adds pending input on input_needed event", () => {
    const state = workbenchReducer(createWorkbenchState(), {
      type: "stream_event",
      event: {
        type: "input_needed",
        input_id: "input-1",
        prompt: "What is your name?",
      },
    });

    expect(state.pendingInputs).toHaveLength(1);
    expect(state.pendingInputs[0]).toEqual({
      input_id: "input-1",
      prompt: "What is your name?",
    });
    expect(state.trace[0].label).toBe("input_needed");
  });

  it("removes pending input on input_submitted action", () => {
    const withInput = workbenchReducer(createWorkbenchState(), {
      type: "stream_event",
      event: {
        type: "input_needed",
        input_id: "input-1",
        prompt: "What is your name?",
      },
    });

    const submitted = workbenchReducer(withInput, {
      type: "input_submitted",
      inputId: "input-1",
    });

    expect(submitted.pendingInputs).toHaveLength(0);
  });

  it("syncs pending interactions from a job state response", () => {
    const state = workbenchReducer(createWorkbenchState(), {
      type: "job_state_synced",
      state: {
        job_id: "job-1",
        run_id: "run-1",
        status: "running",
        event_count: 7,
        pending_approvals: [
          {
            call_id: "call-1",
            name: "fs_write",
            args: { path: "notes.md" },
            reason: "destructive tool requires explicit approval",
          },
        ],
        pending_inputs: [
          {
            input_id: "input-1",
            prompt: "Which branch should I use?",
          },
        ],
      },
    });

    expect(state.activeJobId).toBe("job-1");
    expect(state.activeRunId).toBe("run-1");
    expect(state.busy).toBe(true);
    expect(state.eventCount).toBe(7);
    expect(state.pendingInputs).toEqual([
      {
        input_id: "input-1",
        prompt: "Which branch should I use?",
      },
    ]);
    expect(state.tools[0]).toMatchObject({
      id: "call-1",
      name: "fs_write",
      status: "waiting",
      pendingApproval: {
        call_id: "call-1",
        reason: "destructive tool requires explicit approval",
      },
    });
  });

  it("clears pending interactions when a synced job state is terminal", () => {
    const withPending = workbenchReducer(createWorkbenchState(), {
      type: "stream_event",
      event: {
        type: "tool_call_approval_needed",
        call_id: "call-1",
        name: "fs_write",
        args: { path: "notes.md" },
        reason: "destructive tool requires explicit approval",
      },
    });

    const cancelled = workbenchReducer(
      {
        ...withPending,
        pendingInputs: [
          {
            input_id: "input-1",
            prompt: "Which branch should I use?",
          },
        ],
      },
      {
        type: "job_state_synced",
        state: {
          job_id: "job-1",
          run_id: "run-1",
          status: "cancelled",
          event_count: 8,
          pending_approvals: [],
          pending_inputs: [],
        },
      },
    );

    expect(cancelled.busy).toBe(false);
    expect(cancelled.statusText).toBe("Run cancelled");
    expect(cancelled.pendingInputs).toHaveLength(0);
    expect(cancelled.tools[0]).toMatchObject({
      id: "call-1",
      status: "error",
      details: "Run cancelled",
    });
    expect(cancelled.tools[0].pendingApproval).toBeUndefined();
  });
});
