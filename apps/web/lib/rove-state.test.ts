import { describe, expect, it } from "vitest";

import {
  createWorkbenchState,
  selectTranscriptTimeline,
  workbenchReducer,
} from "./rove-state";
import type { PlanRevision, StreamEvent } from "./rove-types";

const emptyBudgetSnapshot = {
  plan_steps: 0,
  step_attempts: 0,
  model_turns: 0,
  tool_calls: 0,
  plan_revisions: 0,
  model_repairs: 0,
  planner_turns: 0,
  evaluator_turns: 0,
  replanner_turns: 0,
  finalization_turns: 0,
  wall_time_ms: 0,
  total_tokens: 0,
  cost_microunits: 0,
};

function planRevision(overrides: Partial<PlanRevision> = {}): PlanRevision {
  return {
    plan_id: "plan-1",
    revision_id: "revision-0",
    revision: 0,
    created_at: "2026-07-20T00:00:00Z",
    decision_id: "initial-decision",
    remaining_steps: [{ id: "1", title: "Inspect docs", done: false }],
    budget_snapshot: emptyBudgetSnapshot,
    ...overrides,
  };
}

describe("workbenchReducer", () => {
  it("normalizes execution lists omitted from live tool metadata", () => {
    const state = workbenchReducer(createWorkbenchState(), {
      type: "stream_event",
      event: {
        type: "tool_call_completed",
        call_id: "call-input",
        result: {
          call_id: "call-input",
          output: "main",
          metadata: {
            status: "ok",
            risk_level: "low",
            read_only: true,
            workspace_changed: false,
          },
        },
      },
    });

    expect(state.tools[0]?.metadata).toEqual({
      status: "ok",
      risk_level: "low",
      read_only: true,
      affected_paths: [],
      workspace_changed: false,
      diff_summary: [],
    });
  });

  it("keeps canonical message, approval, input, tool-result, and follow-up order", () => {
    let state = workbenchReducer(createWorkbenchState(), {
      type: "job_created",
      jobId: "job-1",
      runId: "run-1",
    });
    const events: Array<{ seq: number; event: StreamEvent }> = [
      {
        seq: 1,
        event: {
          type: "run_started",
          run_id: "run-1",
          job_id: "job-1",
          user_message: "Inspect the workspace",
        },
      },
      {
        seq: 2,
        event: {
          type: "llm_message",
          full: "I will inspect it.",
          usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
        },
      },
      {
        seq: 3,
        event: {
          type: "tool_call_started",
          call_id: "approval-call",
          name: "write_file",
          args: { path: "notes.txt" },
        },
      },
      {
        seq: 4,
        event: {
          type: "tool_call_approval_needed",
          call_id: "approval-call",
          name: "write_file",
          args: { path: "notes.txt" },
          reason: "Writing a file requires approval",
        },
      },
      {
        seq: 5,
        event: {
          type: "input_needed",
          input_id: "input-1",
          prompt: "Which format should I use?",
        },
      },
      {
        seq: 6,
        event: {
          type: "tool_call_started",
          call_id: "read-call",
          name: "read_file",
          args: { path: "README.md" },
        },
      },
      {
        seq: 7,
        event: {
          type: "tool_call_completed",
          call_id: "read-call",
          result: {
            call_id: "read-call",
            output: "read complete",
            mutations: [],
          },
        },
      },
      {
        seq: 8,
        event: {
          type: "llm_message",
          full: "The read is complete.",
          usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
        },
      },
    ];
    for (const stored of events) {
      state = workbenchReducer(state, { type: "stream_event", ...stored });
    }

    const group = selectTranscriptTimeline(state)[0]!;
    expect(group.runId).toBe("run-1");
    expect(
      group.items.map((item) => {
        switch (item.kind) {
          case "message":
            return `message:${item.message.content}`;
          case "tool":
            return `tool:${item.tool.id}:${item.tool.status}`;
          case "input":
            return `input:${item.input.id}`;
        }
      }),
    ).toEqual([
      "message:Inspect the workspace",
      "message:I will inspect it.",
      "tool:approval-call:waiting",
      "input:input-1",
      "tool:read-call:done",
      "message:The read is complete.",
    ]);
    expect(group.items[2]?.entry.eventSeq).toBe(3);

    const replayed = workbenchReducer(state, {
      type: "stream_event",
      ...events[6]!,
    });
    expect(replayed).toBe(state);
    expect(selectTranscriptTimeline(replayed)[0]?.items).toHaveLength(6);
  });

  it("places distinct run completion output after intervening tool activity", () => {
    let state = workbenchReducer(createWorkbenchState(), {
      type: "job_created",
      jobId: "job-1",
      runId: "run-1",
    });
    const events: Array<{ seq: number; event: StreamEvent }> = [
      {
        seq: 1,
        event: {
          type: "run_started",
          run_id: "run-1",
          job_id: "job-1",
          user_message: "Inspect the workspace",
        },
      },
      {
        seq: 2,
        event: {
          type: "llm_message",
          full: "I will inspect it.",
          usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
        },
      },
      {
        seq: 3,
        event: {
          type: "tool_call_started",
          call_id: "read-call",
          name: "read_file",
          args: { path: "README.md" },
        },
      },
      {
        seq: 4,
        event: {
          type: "tool_call_completed",
          call_id: "read-call",
          result: { call_id: "read-call", output: "read complete", mutations: [] },
        },
      },
      {
        seq: 5,
        event: {
          type: "run_completed",
          reason: "model_error",
          output: "The run stopped after the read failed.",
        },
      },
    ];
    for (const stored of events) {
      state = workbenchReducer(state, { type: "stream_event", ...stored });
    }

    const group = selectTranscriptTimeline(state)[0]!;
    expect(
      group.items.map((item) => {
        switch (item.kind) {
          case "message":
            return `message:${item.message.content}`;
          case "tool":
            return `tool:${item.tool.id}`;
          case "input":
            return `input:${item.input.id}`;
        }
      }),
    ).toEqual([
      "message:Inspect the workspace",
      "message:I will inspect it.",
      "tool:read-call",
      "message:The run stopped after the read failed.",
    ]);
    expect(group.items.at(-1)?.entry.eventSeq).toBe(5);
    expect(state.messages.map((message) => message.content)).toEqual([
      "Inspect the workspace",
      "I will inspect it.",
      "The run stopped after the read failed.",
    ]);
  });

  it("sorts a recovered earlier sequence into its canonical position without replay duplicates", () => {
    let state = workbenchReducer(createWorkbenchState(), {
      type: "job_created",
      jobId: "job-1",
      runId: "run-1",
    });
    state = workbenchReducer(state, {
      type: "stream_event",
      seq: 1,
      event: {
        type: "run_started",
        run_id: "run-1",
        job_id: "job-1",
        user_message: "Question",
      },
    });
    state = workbenchReducer(state, {
      type: "stream_event",
      seq: 3,
      event: {
        type: "tool_call_started",
        call_id: "call-1",
        name: "read_file",
        args: { path: "README.md" },
      },
    });
    state = workbenchReducer(state, {
      type: "stream_event",
      seq: 4,
      event: {
        type: "tool_call_completed",
        call_id: "call-1",
        result: { call_id: "call-1", output: "done", mutations: [] },
      },
    });
    const recovered = workbenchReducer(state, {
      type: "stream_event",
      seq: 2,
      event: {
        type: "llm_message",
        full: "Earlier assistant turn",
        usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
      },
    });

    expect(
      selectTranscriptTimeline(recovered)[0]?.items.map((item) => [
        item.kind,
        item.entry.eventSeq,
      ]),
    ).toEqual([
      ["message", 1],
      ["message", 2],
      ["tool", 3],
    ]);
    const replayed = workbenchReducer(recovered, {
      type: "stream_event",
      seq: 2,
      event: {
        type: "llm_message",
        full: "Earlier assistant turn",
        usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
      },
    });
    expect(replayed).toBe(recovered);
    expect(selectTranscriptTimeline(replayed)[0]?.items).toHaveLength(3);
  });

  it("hydrates user messages from run_started events and ignores duplicate seq values", () => {
    const runStarted = {
      type: "run_started",
      run_id: "run-1",
      job_id: "job-1",
      user_message: "hello",
    } as const;

    const started = workbenchReducer(createWorkbenchState(), {
      type: "stream_event",
      seq: 1,
      event: runStarted,
    });
    const replayed = workbenchReducer(started, {
      type: "stream_event",
      seq: 1,
      event: runStarted,
    });

    expect(replayed.eventCount).toBe(1);
    expect(replayed.messages).toEqual([
      expect.objectContaining({
        role: "user",
        content: "hello",
        status: "final",
      }),
    ]);
    expect(replayed.trace).toHaveLength(1);
  });

  it("tracks resumed source run identity from create and synced job state", () => {
    const created = workbenchReducer(createWorkbenchState(), {
      type: "job_created",
      jobId: "job-1",
      runId: "run-2",
      resumedFromRunId: "run-1",
    });

    expect(created.activeJobId).toBe("job-1");
    expect(created.activeRunId).toBe("run-2");
    expect(created.resumedFromRunId).toBe("run-1");
    expect(created.lastSignal).toBe("Resumed run");

    const synced = workbenchReducer(created, {
      type: "job_state_synced",
      state: {
        job_id: "job-1",
        run_id: "run-2",
        resumed_from_run_id: "run-1",
        status: "done",
        event_count: 1,
        events: [],
        pending_approvals: [],
        pending_inputs: [],
      },
    });

    expect(synced.resumedFromRunId).toBe("run-1");
    expect(synced.busy).toBe(false);
  });

  it("clears resumed source identity on reset and fresh jobs", () => {
    const resumed = workbenchReducer(createWorkbenchState(), {
      type: "job_created",
      jobId: "job-1",
      runId: "run-2",
      resumedFromRunId: "run-1",
    });

    const fresh = workbenchReducer(resumed, {
      type: "job_created",
      jobId: "job-2",
      runId: "run-3",
    });

    expect(fresh.resumedFromRunId).toBeNull();
    expect(workbenchReducer(resumed, { type: "reset" }).resumedFromRunId).toBeNull();
  });

  it("does not append duplicate message, tool, trace, input, or plan state on replay", () => {
    const withChunk = workbenchReducer(createWorkbenchState(), {
      type: "stream_event",
      seq: 2,
      event: { type: "llm_chunk", delta: "Hi" },
    });
    const duplicateChunk = workbenchReducer(withChunk, {
      type: "stream_event",
      seq: 2,
      event: { type: "llm_chunk", delta: "Hi" },
    });
    expect(duplicateChunk.messages).toHaveLength(1);
    expect(duplicateChunk.messages[0].content).toBe("Hi");

    const withTool = workbenchReducer(duplicateChunk, {
      type: "stream_event",
      seq: 3,
      event: {
        type: "tool_call_started",
        call_id: "call-1",
        name: "echo",
        args: { text: "hello" },
      },
    });
    const duplicateTool = workbenchReducer(withTool, {
      type: "stream_event",
      seq: 3,
      event: {
        type: "tool_call_started",
        call_id: "call-1",
        name: "echo",
        args: { text: "hello" },
      },
    });
    expect(duplicateTool.tools).toHaveLength(1);
    expect(duplicateTool.trace).toHaveLength(1);

    const withPlan = workbenchReducer(duplicateTool, {
      type: "stream_event",
      seq: 4,
      event: {
        type: "plan_created",
        plan: {
          goal: "test",
          current_step: 0,
          steps: [{ id: "1", title: "Check", done: false }],
        },
      },
    });
    const duplicatePlan = workbenchReducer(withPlan, {
      type: "stream_event",
      seq: 4,
      event: {
        type: "plan_created",
        plan: {
          goal: "test",
          current_step: 0,
          steps: [{ id: "1", title: "Check", done: false }],
        },
      },
    });
    expect(duplicatePlan.plan?.steps).toHaveLength(1);
    expect(duplicatePlan.trace).toHaveLength(2);

    const withInput = workbenchReducer(duplicatePlan, {
      type: "stream_event",
      seq: 5,
      event: {
        type: "input_needed",
        input_id: "input-1",
        prompt: "Which branch?",
      },
    });
    const duplicateInput = workbenchReducer(withInput, {
      type: "stream_event",
      seq: 5,
      event: {
        type: "input_needed",
        input_id: "input-1",
        prompt: "Which branch?",
      },
    });
    expect(duplicateInput.pendingInputs).toHaveLength(1);
    expect(duplicateInput.trace).toHaveLength(3);
  });

  it("projects and deduplicates plan decisions and immutable revisions", () => {
    const firstPlan = workbenchReducer(createWorkbenchState(), {
      type: "stream_event",
      seq: 1,
      event: {
        type: "plan_created",
        plan: {
          goal: "fix docs",
          current_step: 0,
          steps: [{ id: "1", title: "Inspect docs", done: false }],
        },
        plan_id: "plan-1",
        plan_revision_id: "revision-0",
        revision: 0,
        plan_revision: planRevision(),
      },
    });

    const decided = workbenchReducer(firstPlan, {
      type: "stream_event",
      seq: 2,
      event: {
        type: "plan_decision",
        record: {
          trigger_step_record_id: "record-1",
          decided_at: "2026-07-20T00:00:01Z",
          decision: {
            decision_id: "decision-1",
            kind: "replace_remaining",
            safe_reason_codes: ["recoverable_step_failure"],
            safe_summary: "Replace the failed remaining work.",
            remaining_work_requirements: ["Use a safe alternative."],
          },
        },
      },
    });
    const replayedDecision = workbenchReducer(decided, {
      type: "stream_event",
      seq: 3,
      event: {
        type: "plan_decision",
        record: decided.planDecisions[0],
      },
    });
    const childRevision = planRevision({
      revision_id: "revision-1",
      parent_revision_id: "revision-0",
      revision: 1,
      created_at: "2026-07-20T00:00:02Z",
      trigger_step_record_id: "record-1",
      decision_id: "decision-1",
      remaining_steps: [{ id: "2", title: "Inspect docs without a tool", done: false }],
    });
    const replanned = workbenchReducer(replayedDecision, {
      type: "stream_event",
      seq: 4,
      event: {
        type: "plan_revised",
        plan: {
          goal: "fix docs",
          current_step: 0,
          steps: [{ id: "2", title: "Inspect docs without a tool", done: false }],
        },
        revision: childRevision,
      },
    });
    const replayedRevision = workbenchReducer(replanned, {
      type: "stream_event",
      seq: 5,
      event: {
        type: "plan_revised",
        plan: replanned.plan!,
        revision: childRevision,
      },
    });

    expect(replayedRevision.plan?.steps).toEqual([
      { id: "2", title: "Inspect docs without a tool", done: false },
    ]);
    expect(replayedRevision.plan?.current_step).toBe(0);
    expect(replayedRevision.planDecisions).toHaveLength(1);
    expect(replayedRevision.planRevisions.map((revision) => revision.revision)).toEqual([0, 1]);
    expect(
      replayedRevision.trace.filter((entry) => entry.label === "plan_decision"),
    ).toHaveLength(1);
    expect(
      replayedRevision.trace.filter((entry) => entry.label === "plan_revised"),
    ).toHaveLength(1);
  });

  it("projects step_result records without duplicating the visible trace", () => {
    const event: StreamEvent = {
      type: "step_result",
      record: {
        record_id: "record-1",
        plan_id: "plan-1",
        plan_revision_id: "revision-1",
        step_id: "1",
        attempt: 1,
        status: "succeeded",
        started_at: "2026-07-20T00:00:00Z",
        finished_at: "2026-07-20T00:00:01Z",
        summary: "inspection complete",
        completion_basis: "model_conclusion",
        model_turns_used: 1,
        tool_calls_used: 0,
        token_usage: {
          prompt_tokens: 1,
          completion_tokens: 1,
          total_tokens: 2,
        },
      },
    };
    const recorded = workbenchReducer(createWorkbenchState(), {
      type: "stream_event",
      seq: 1,
      event,
    });
    const replayedWithNewSequence = workbenchReducer(recorded, {
      type: "stream_event",
      seq: 2,
      event,
    });

    expect(replayedWithNewSequence.stepRecords).toEqual([event.record]);
    // First delivery projects one visible step_result row; replay must not
    // duplicate either the structured record or the trace entry.
    expect(replayedWithNewSequence.trace).toHaveLength(1);
    expect(replayedWithNewSequence.trace[0]?.label).toBe("step_result");
  });

  it("hydrates recoverable UI state from sequenced job state events", () => {
    const state = workbenchReducer(createWorkbenchState(), {
      type: "job_state_synced",
      state: {
        job_id: "job-1",
        run_id: "run-1",
        status: "running",
        event_count: 6,
        events: [
          {
            seq: 1,
            event: {
              type: "run_started",
              job_id: "job-1",
              run_id: "run-1",
              user_message: "summarize",
            },
          },
          {
            seq: 2,
            event: {
              type: "llm_message",
              full: "summary",
              usage: {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: 2,
              },
            },
          },
          {
            seq: 3,
            event: {
              type: "plan_created",
              plan: {
                goal: "summarize",
                current_step: 0,
                steps: [{ id: "1", title: "Read", done: false }],
              },
            },
          },
          {
            seq: 4,
            event: {
              type: "tool_call_started",
              call_id: "call-1",
              name: "echo",
              args: { text: "ok" },
            },
          },
          {
            seq: 5,
            event: {
              type: "tool_call_completed",
              call_id: "call-1",
              result: {
                call_id: "call-1",
                output: "ok",
              },
            },
          },
          {
            seq: 6,
            event: {
              type: "input_needed",
              input_id: "input-1",
              prompt: "Continue?",
            },
          },
        ],
        pending_approvals: [
          {
            call_id: "call-2",
            name: "write_file",
            args: { path: "notes.md" },
            reason: "destructive tool requires explicit approval",
          },
        ],
        pending_inputs: [
          {
            input_id: "input-1",
            prompt: "Continue?",
          },
        ],
      },
    });

    expect(state.activeJobId).toBe("job-1");
    expect(state.activeRunId).toBe("run-1");
    expect(state.eventCount).toBe(6);
    expect(state.messages.map((message) => message.content)).toEqual([
      "summarize",
      "summary",
    ]);
    expect(state.plan?.goal).toBe("summarize");
    expect(state.tools).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ id: "call-1", status: "done", details: "ok" }),
        expect.objectContaining({ id: "call-2", status: "waiting" }),
      ]),
    );
    expect(state.pendingInputs).toEqual([
      {
        input_id: "input-1",
        prompt: "Continue?",
      },
    ]);
    expect(state.trace.length).toBeGreaterThanOrEqual(4);
  });

  it("does not let a stale job snapshot overwrite newer stream events", () => {
    const created = workbenchReducer(createWorkbenchState(), {
      type: "job_created",
      jobId: "job-1",
      runId: "run-1",
    });
    const completed = workbenchReducer(created, {
      type: "stream_event",
      seq: 12,
      event: {
        type: "run_completed",
        reason: "final",
        output: "main",
      },
    });

    const synced = workbenchReducer(completed, {
      type: "job_state_synced",
      state: {
        job_id: "job-1",
        run_id: "run-1",
        status: "running",
        event_count: 9,
        events: [],
        pending_approvals: [],
        pending_inputs: [],
      },
    });

    expect(synced).toBe(completed);
    expect(synced.eventCount).toBe(12);
    expect(synced.busy).toBe(false);
    expect(synced.statusText).toBe("Run completed: final");
    expect(synced.lastSignal).toBe("Run completed");
  });

  it("preserves state when syncing prompt-built events from job history", () => {
    const state = workbenchReducer(createWorkbenchState(), {
      type: "job_state_synced",
      state: {
        job_id: "job-1",
        run_id: "run-1",
        status: "done",
        event_count: 3,
        events: [
          {
            seq: 1,
            event: {
              type: "run_started",
              job_id: "job-1",
              run_id: "run-1",
              user_message: "hello",
            },
          },
          {
            seq: 2,
            event: {
              type: "prompt_built",
              metadata: {
                prompt_hash: "sha256:prompt",
                stable_prefix_hash: "sha256:prefix",
                workspace_fingerprint: "sha256:workspace",
                tool_signature: "sha256:tools",
                token_estimate: 42,
                included_history_messages: 1,
                dropped_history_messages: 0,
                prompt_cache_key: "sha256:cache",
              },
            },
          },
          {
            seq: 3,
            event: {
              type: "run_completed",
              reason: "final",
              output: "done",
            },
          },
        ],
        pending_approvals: [],
        pending_inputs: [],
      },
    });

    expect(state.activeJobId).toBe("job-1");
    expect(state.statusText).toBe("Run completed");
    expect(state.seenEventSeqs).toEqual([1, 2, 3]);
    expect(state.trace.map((entry) => entry.label)).toContain("prompt_built");
  });

  it("renders every runtime stream event variant into recoverable UI state", () => {
    const events: StreamEvent[] = [
      {
        type: "run_started",
        job_id: "job-1",
        run_id: "run-1",
        user_message: "summarize",
      },
      {
        type: "model_status",
        status: "thinking",
        message: "Model is thinking",
      },
      { type: "llm_chunk", delta: "sum" },
      {
        type: "llm_message",
        full: "summary",
        usage: {
          prompt_tokens: 1,
          completion_tokens: 1,
          total_tokens: 2,
        },
        tool_calls: [
          {
            id: "toolu-1",
            name: "echo",
            args: { text: "ok" },
          },
        ],
      },
      {
        type: "tool_call_started",
        call_id: "call-1",
        tool_use_id: "toolu-1",
        name: "echo",
        args: { text: "ok" },
      },
      {
        type: "tool_call_approval_needed",
        call_id: "call-2",
        name: "write_file",
        args: { path: "notes.md" },
        reason: "destructive tool requires explicit approval",
      },
      {
        type: "tool_call_completed",
        call_id: "call-1",
        result: {
          call_id: "call-1",
          output: "ok",
          mutations: [
            {
              path: "notes.md",
              operation: "create",
              diff: "+ok",
            },
          ],
        },
      },
      {
        type: "tool_call_failed",
        call_id: "call-2",
        error: {
          code: "rejected",
          reason: "user rejected",
        },
      },
      {
        type: "input_needed",
        input_id: "input-1",
        prompt: "Continue?",
      },
      {
        type: "plan_created",
        plan: {
          goal: "summarize",
          current_step: 0,
          steps: [{ id: "1", title: "Read", done: false }],
        },
      },
      {
        type: "plan_step_started",
        index: 0,
        step: { id: "1", title: "Read", done: false },
      },
      {
        type: "step_result",
        record: {
          record_id: "record-1",
          plan_id: "plan-1",
          plan_revision_id: "revision-0",
          step_id: "1",
          attempt: 1,
          status: "succeeded",
          started_at: "2026-07-20T00:00:00Z",
          finished_at: "2026-07-20T00:00:01Z",
          summary: "Read",
          completion_basis: "model_conclusion",
          model_turns_used: 1,
          tool_calls_used: 0,
          token_usage: {
            prompt_tokens: 1,
            completion_tokens: 1,
            total_tokens: 2,
          },
        },
      },
      {
        type: "prompt_compacted",
        summary: "Earlier context summarized",
        state: {
          mode: "model_generated",
          auto_triggered: true,
          degraded: false,
          consecutive_failures: 0,
          circuit_open: false,
          model: "fake",
          prompt_version: "rove.compaction.v1",
          source_message_count: 4,
        },
      },
      {
        type: "run_completed",
        reason: "final",
        output: "done",
      },
    ];

    const state = events.reduce(
      (current, event, index) =>
        workbenchReducer(current, {
          type: "stream_event",
          seq: index + 1,
          event,
        }),
      createWorkbenchState(),
    );

    expect(state.activeJobId).toBe("job-1");
    expect(state.activeRunId).toBe("run-1");
    expect(state.busy).toBe(false);
    expect(state.statusText).toBe("Run completed: final");
    expect(state.messages.map((message) => message.content)).toEqual([
      "summarize",
      "summary",
      "done",
    ]);
    expect(state.tools).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ id: "call-1", status: "done" }),
        expect.objectContaining({ id: "call-2", status: "error" }),
      ]),
    );
    expect(state.pendingInputs).toEqual([]);
    expect(state.transcriptInputs).toEqual([
      expect.objectContaining({
        id: "input-1",
        prompt: "Continue?",
        status: "closed",
      }),
    ]);
    expect(state.plan?.current_step).toBe(1);
    expect(state.trace.map((entry) => entry.label)).toEqual(
      expect.arrayContaining([
        "run_started",
        "model_status",
        "tool_call_started",
        "tool_call_approval_needed",
        "tool_call_completed",
        "tool_call_failed",
        "input_needed",
        "plan_created",
        "plan_step_started",
        "step_result",
        "prompt_compacted",
        "run_completed",
      ]),
    );
  });

  it("marks an approved waiting tool as running and clears the pending approval", () => {
    const withWaitingTool = workbenchReducer(createWorkbenchState(), {
      type: "stream_event",
      event: {
        type: "tool_call_approval_needed",
        call_id: "call-1",
        name: "write_file",
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
        name: "write_file",
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
        name: "run_shell",
        args: { command: "rm -rf /tmp/test" },
        reason: "destructive tool requires explicit approval",
      },
    });

    expect(state.tools[0]).toMatchObject({
      id: "call-2",
      status: "waiting",
      pendingApproval: {
        call_id: "call-2",
        name: "run_shell",
        reason: "destructive tool requires explicit approval",
      },
    });
  });

  it("renders model status events in status text and trace", () => {
    const state = workbenchReducer(createWorkbenchState(), {
      type: "stream_event",
      event: {
        type: "model_status",
        status: "thinking",
        message: "Model is thinking",
      },
    });

    expect(state.statusText).toBe("Model is thinking");
    expect(state.trace[0]).toMatchObject({
      label: "model_status",
      detail: "Model is thinking",
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
    expect(submitted.transcriptInputs[0]).toMatchObject({
      id: "input-1",
      status: "submitted",
    });
    expect(selectTranscriptTimeline(submitted)[0]?.items[0]).toMatchObject({
      kind: "input",
      input: { id: "input-1", status: "submitted" },
    });
  });

  it("syncs pending interactions from a job state response", () => {
    const state = workbenchReducer(createWorkbenchState(), {
      type: "job_state_synced",
      state: {
        job_id: "job-1",
        run_id: "run-1",
        status: "running",
        event_count: 7,
        events: [],
        pending_approvals: [
          {
            call_id: "call-1",
            name: "write_file",
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
      name: "write_file",
      status: "waiting",
      pendingApproval: {
        call_id: "call-1",
        reason: "destructive tool requires explicit approval",
      },
    });
  });

  it("closes a missing pending input from a running job snapshot", () => {
    const created = workbenchReducer(createWorkbenchState(), {
      type: "job_created",
      jobId: "job-1",
      runId: "run-1",
    });
    const waiting = workbenchReducer(created, {
      type: "stream_event",
      seq: 1,
      event: {
        type: "input_needed",
        input_id: "input-1",
        prompt: "Which branch should I use?",
      },
    });

    const synced = workbenchReducer(waiting, {
      type: "job_state_synced",
      state: {
        job_id: "job-1",
        run_id: "run-1",
        status: "running",
        event_count: 1,
        events: [],
        pending_approvals: [],
        pending_inputs: [],
      },
    });

    expect(synced.busy).toBe(true);
    expect(synced.pendingInputs).toEqual([]);
    expect(synced.transcriptInputs).toEqual([
      expect.objectContaining({ id: "input-1", status: "closed" }),
    ]);
    expect(selectTranscriptTimeline(synced)[0]?.items[0]).toMatchObject({
      kind: "input",
      input: { id: "input-1", status: "closed" },
    });
  });

  it("clears pending interactions when a synced job state is terminal", () => {
    const withPending = workbenchReducer(createWorkbenchState(), {
      type: "stream_event",
      event: {
        type: "tool_call_approval_needed",
        call_id: "call-1",
        name: "write_file",
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
          events: [],
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

  it("treats interrupted job state as terminal after API restart", () => {
    const state = workbenchReducer(createWorkbenchState(), {
      type: "job_state_synced",
      state: {
        job_id: "job-1",
        run_id: "run-1",
        status: "interrupted",
        event_count: 3,
        events: [],
        pending_approvals: [],
        pending_inputs: [],
      },
    });

    expect(state.busy).toBe(false);
    expect(state.statusText).toBe("Run interrupted");
  });
});
