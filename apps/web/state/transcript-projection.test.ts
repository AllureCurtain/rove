import { describe, expect, it } from "vitest";

import type {
  ProductTranscriptPartialReason,
  ProductTranscriptResponse,
  ProductTranscriptRunSegment,
} from "../product/product-api-types";
import {
  describeTranscriptPartialReason,
  projectProductTranscript,
} from "./transcript-projection";
import { workbenchReducer } from "../lib/rove-state";

describe("product transcript projection", () => {
  it("replays each run ordinal through the canonical reducer despite repeated seqs", () => {
    const transcript: ProductTranscriptResponse = {
      product_session_id: "product-session",
      workspace_id: "workspace",
      status: "complete",
      partial_reasons: [],
      segments: [
        completedSegment(1, "job-1", "run-1", "First question", "First answer"),
        completedSegment(2, "job-2", "run-2", "Second question", "Second answer", "run-1"),
      ],
    };

    const state = projectProductTranscript(transcript);

    expect(state.messages.map((message) => [message.role, message.content])).toEqual([
      ["user", "First question"],
      ["assistant", "First answer"],
      ["user", "Second question"],
      ["assistant", "Second answer"],
    ]);
    expect(state.activeJobId).toBe("job-2");
    expect(state.activeRunId).toBe("run-2");
    expect(state.resumedFromRunId).toBe("run-1");
    expect(state.seenEventSeqs).toEqual([1, 2, 3, 4]);
    expect(state.busy).toBe(false);
  });

  it("keeps identical user text from separate run ordinals", () => {
    const state = projectProductTranscript({
      product_session_id: "product-session",
      workspace_id: "workspace",
      status: "complete",
      partial_reasons: [],
      segments: [
        completedSegment(1, "job-1", "run-1", "Continue", "First answer"),
        completedSegment(
          2,
          "job-2",
          "run-2",
          "Continue",
          "Second answer",
          "run-1",
        ),
      ],
    });

    expect(
      state.messages.filter(
        (message) => message.role === "user" && message.content === "Continue",
      ),
    ).toHaveLength(2);
  });

  it("retains terminal tool cards from earlier run ordinals", () => {
    const state = projectProductTranscript({
      product_session_id: "product-session",
      workspace_id: "workspace",
      status: "complete",
      partial_reasons: [],
      segments: [
        completedToolSegment(1, "job-1", "run-1", "call-1", "read_file"),
        completedToolSegment(
          2,
          "job-2",
          "run-2",
          "call-2",
          "list_files",
          "run-1",
        ),
      ],
    });

    expect(state.tools.map((tool) => [tool.id, tool.name, tool.status])).toEqual([
      ["call-2", "list_files", "done"],
    ]);
    expect(
      state.historicalTools.map((tool) => [tool.id, tool.name, tool.status]),
    ).toEqual([["call-1", "read_file", "done"]]);
  });

  it("keeps the latest running segment ready for focused reattachment", () => {
    const segment = completedSegment(
      1,
      "job-live",
      "run-live",
      "Still working",
      "draft",
    );
    segment.run_status = "running";
    segment.events = segment.events.slice(0, 2);
    segment.observed_through_seq = 2;
    segment.last_event_seq = 2;
    const state = projectProductTranscript({
      product_session_id: "product-session",
      workspace_id: "workspace",
      status: "complete",
      partial_reasons: [],
      segments: [segment],
    });

    expect(state.busy).toBe(true);
    expect(state.activeJobId).toBe("job-live");
    expect(state.messages[1]).toMatchObject({
      role: "assistant",
      content: "draft",
      status: "streaming",
    });

    const continued = workbenchReducer(state, {
      type: "stream_event",
      seq: 3,
      event: { type: "llm_chunk", delta: " continued" },
    });
    expect(
      continued.messages.filter((message) => message.role === "assistant"),
    ).toEqual([
      expect.objectContaining({
        content: "draft continued",
        status: "streaming",
      }),
    ]);
  });

  it("accepts sequence one when the catalog follows a newer job", () => {
    let state = projectProductTranscript({
      product_session_id: "product-session",
      workspace_id: "workspace",
      status: "complete",
      partial_reasons: [],
      segments: [
        completedSegment(1, "job-old", "run-old", "Old question", "Old answer"),
      ],
    });

    state = workbenchReducer(state, {
      type: "prepare_job_attachment",
      jobId: "job-live",
    });
    state = workbenchReducer(state, {
      type: "stream_event",
      seq: 1,
      event: {
        type: "run_started",
        job_id: "job-live",
        run_id: "run-live",
        user_message: "New question",
      },
    });
    state = workbenchReducer(state, {
      type: "stream_event",
      seq: 2,
      event: { type: "llm_chunk", delta: "New draft" },
    });

    expect(state.activeJobId).toBe("job-live");
    expect(state.seenEventSeqs).toEqual([1, 2]);
    expect(state.messages.map((message) => [message.content, message.status])).toEqual([
      ["Old question", "final"],
      ["Old answer", "final"],
      ["New question", "final"],
      ["New draft", "streaming"],
    ]);
  });

  it("uses an explicit report fallback without claiming canonical completeness", () => {
    const state = projectProductTranscript({
      product_session_id: "product-session",
      workspace_id: "workspace",
      status: "partial",
      partial_reasons: [{ code: "cleaned_history", run_ordinal: 1 }],
      segments: [
        {
          binding: binding(1, "job-1", "run-1"),
          run_status: "done",
          observed_through_seq: 0,
          last_event_seq: 0,
          events: [],
          fallback: {
            source: "report",
            status: "done",
            summary: "A bounded report summary remains.",
          },
        },
      ],
    });

    expect(state.messages).toEqual([
      {
        id: "fallback-run-1",
        role: "assistant",
        content: "Report summary: A bounded report summary remains.",
        status: "final",
      },
    ]);
  });

  it("keeps a report fallback when canonical events stop at a partial draft", () => {
    const segment = completedSegment(
      1,
      "job-1",
      "run-1",
      "Question",
      "Partial draft",
    );
    segment.events = segment.events.slice(0, 2);
    segment.observed_through_seq = 2;
    segment.last_event_seq = 4;
    segment.fallback = {
      source: "report",
      status: "done",
      summary: "Complete report answer",
    };

    const state = projectProductTranscript({
      product_session_id: "product-session",
      workspace_id: "workspace",
      status: "partial",
      partial_reasons: [
        {
          code: "missing_event_range",
          run_ordinal: 1,
          expected_seq: 3,
          observed_seq: 4,
        },
      ],
      segments: [segment],
    });

    expect(state.messages.map((message) => [message.content, message.status])).toEqual([
      ["Question", "final"],
      ["Partial draft", "final"],
      ["Report summary: Complete report answer", "final"],
    ]);
  });

  it("turns typed partial reasons into stable user-visible detail", () => {
    const reason: ProductTranscriptPartialReason = {
      code: "missing_event_range",
      run_ordinal: 2,
      run_id: "01J00000000000000000000002",
      expected_seq: 4,
      observed_seq: 6,
    };
    expect(describeTranscriptPartialReason(reason)).toContain("run 2");
    expect(describeTranscriptPartialReason(reason)).toContain("canonical event range");
    expect(describeTranscriptPartialReason(reason)).toContain("Expected event 4, observed 6");
  });
});

function completedSegment(
  ordinal: number,
  jobId: string,
  runId: string,
  question: string,
  answer: string,
  resumedFromRunId?: string,
): ProductTranscriptRunSegment {
  return {
    binding: binding(ordinal, jobId, runId, resumedFromRunId),
    run_status: "done",
    observed_through_seq: 4,
    last_event_seq: 4,
    events: [
      {
        seq: 1,
        event: {
          type: "run_started",
          job_id: jobId,
          run_id: runId,
          user_message: question,
        },
      },
      { seq: 2, event: { type: "llm_chunk", delta: answer } },
      {
        seq: 3,
        event: {
          type: "llm_message",
          full: answer,
          usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
        },
      },
      {
        seq: 4,
        event: { type: "run_completed", reason: "final", output: answer },
      },
    ],
  };
}

function completedToolSegment(
  ordinal: number,
  jobId: string,
  runId: string,
  callId: string,
  toolName: string,
  resumedFromRunId?: string,
): ProductTranscriptRunSegment {
  return {
    binding: binding(ordinal, jobId, runId, resumedFromRunId),
    run_status: "done",
    observed_through_seq: 6,
    last_event_seq: 6,
    events: [
      {
        seq: 1,
        event: {
          type: "run_started",
          job_id: jobId,
          run_id: runId,
          user_message: `Use ${toolName}`,
        },
      },
      {
        seq: 2,
        event: {
          type: "tool_call_started",
          call_id: callId,
          name: toolName,
          args: { path: "." },
        },
      },
      {
        seq: 3,
        event: {
          type: "tool_call_completed",
          call_id: callId,
          result: {
            call_id: callId,
            output: `${toolName} complete`,
            metadata: {
              status: "ok",
              risk_level: "low",
              read_only: true,
              affected_paths: [],
              workspace_changed: false,
              diff_summary: [],
            },
          },
        },
      },
      { seq: 4, event: { type: "llm_chunk", delta: "Done" } },
      {
        seq: 5,
        event: {
          type: "llm_message",
          full: "Done",
          usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
        },
      },
      {
        seq: 6,
        event: { type: "run_completed", reason: "final", output: "Done" },
      },
    ],
  };
}

function binding(
  ordinal: number,
  jobId: string,
  runId: string,
  resumedFromRunId?: string,
) {
  return {
    product_session_id: "product-session",
    ordinal,
    runtime_session_id: "runtime-session",
    runtime_job_id: jobId,
    runtime_run_id: runId,
    resumed_from_run_id: resumedFromRunId,
    bound_at: "2026-07-26T00:00:00.000Z",
  };
}
