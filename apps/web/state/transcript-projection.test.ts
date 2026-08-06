import { describe, expect, it } from "vitest";

import type {
  ProductTranscriptPartialReason,
  ProductTranscriptResponse,
  ProductTranscriptRunSegment,
} from "../product/product-api-types";
import {
  describeTranscriptPartialReason,
  projectProductTranscript,
  toWorkbenchStreamEvent,
} from "./transcript-projection";
import {
  selectTranscriptTimeline,
  workbenchReducer,
} from "../lib/rove-state";

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

  it("marks a fork prefix as read-only inherited history", () => {
    const inherited = completedSegment(
      1,
      "parent-job",
      "parent-run",
      "Parent question",
      "Parent answer",
    );
    inherited.inherited = true;
    inherited.source_product_session_id = "parent-product-session";
    const local = completedSegment(
      2,
      "child-job",
      "child-run",
      "Child question",
      "Child answer",
    );

    const timeline = selectTranscriptTimeline(
      projectProductTranscript({
        product_session_id: "child-product-session",
        workspace_id: "workspace",
        status: "complete",
        partial_reasons: [],
        segments: [inherited, local],
      }),
    );

    expect(
      timeline.map((group) => [group.runId, group.inherited, group.sourceSessionId]),
    ).toEqual([
      ["parent-run", true, "parent-product-session"],
      ["child-run", false, null],
    ]);
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
    const timeline = selectTranscriptTimeline(state);
    expect(timeline.map((group) => [group.runOrdinal, group.runId])).toEqual([
      [1, "run-1"],
      [2, "run-2"],
    ]);
    expect(
      timeline.map((group) =>
        group.items.map((item) => {
          switch (item.kind) {
            case "message":
              return item.message.content;
            case "tool":
              return item.tool.id;
            case "input":
              return item.input.id;
          }
        }),
      ),
    ).toEqual([
      ["Use read_file", "call-1", "Done"],
      ["Use list_files", "call-2", "Done"],
    ]);
  });

  it("retains structured tool, usage, and context facts during canonical restore", () => {
    const promptBuild = {
      prompt_hash: "prompt-hash",
      stable_prefix_hash: "stable-prefix",
      workspace_fingerprint: "workspace-fingerprint",
      tool_signature: "tool-signature",
      token_estimate: 1234,
      included_history_messages: 8,
      dropped_history_messages: 2,
      prompt_cache_key: "prompt-cache-key",
    };
    const promptCompaction = {
      mode: "model_generated" as const,
      auto_triggered: true,
      degraded: false,
      consecutive_failures: 0,
      circuit_open: false,
      model: "fake",
      prompt_version: "rove.compaction.v1",
      source_message_count: 10,
    };
    const segment: ProductTranscriptRunSegment = {
      binding: binding(1, "job-evidence", "run-evidence"),
      inherited: false,
      run_status: "done",
      observed_through_seq: 7,
      last_event_seq: 7,
      events: [
        {
          seq: 1,
          event: {
            type: "run_started",
            job_id: "job-evidence",
            run_id: "run-evidence",
            user_message: "Inspect canonical evidence",
          },
        },
        { seq: 2, event: { type: "prompt_compacted", summary: "bounded", state: promptCompaction } },
        { seq: 3, event: { type: "prompt_built", metadata: promptBuild } },
        {
          seq: 4,
          event: {
            type: "tool_call_started",
            call_id: "call-evidence",
            name: "write_file",
            args: { path: "notes.md", content: "canonical" },
          },
        },
        {
          seq: 5,
          event: {
            type: "tool_call_completed",
            call_id: "call-evidence",
            result: {
              call_id: "call-evidence",
              output: "wrote notes.md",
              mutations: [
                { path: "notes.md", operation: "create", diff: "+canonical" },
              ],
              metadata: {
                status: "ok",
                risk_level: "high",
                read_only: false,
                affected_paths: ["notes.md"],
                workspace_changed: true,
                diff_summary: ["notes.md created"],
              },
            },
          },
        },
        {
          seq: 6,
          event: {
            type: "llm_message",
            full: "Canonical evidence retained.",
            usage: {
              prompt_tokens: 21,
              completion_tokens: 5,
              total_tokens: 26,
              cached_tokens: 3,
            },
          },
        },
        { seq: 7, event: { type: "run_completed", reason: "final", output: "Canonical evidence retained." } },
      ],
    };

    const state = projectProductTranscript({
      product_session_id: "product-session",
      workspace_id: "workspace",
      status: "complete",
      partial_reasons: [],
      segments: [segment],
    });

    expect(state.tools).toEqual([
      expect.objectContaining({
        id: "call-evidence",
        args: { path: "notes.md", content: "canonical" },
        output: "wrote notes.md",
        mutations: [{ path: "notes.md", operation: "create", diff: "+canonical" }],
        metadata: expect.objectContaining({
          status: "ok",
          risk_level: "high",
          workspace_changed: true,
        }),
      }),
    ]);
    expect(state.runUsage).toEqual({
      prompt_tokens: 21,
      completion_tokens: 5,
      total_tokens: 26,
      cached_tokens: 3,
    });
    expect(state.promptBuild).toEqual(promptBuild);
    expect(state.promptCompaction).toEqual(promptCompaction);
    expect(
      state.messages.find((message) => message.role === "assistant"),
    ).toMatchObject({
      usage: state.runUsage,
      promptBuild,
      promptCompaction,
    });

    const replayed = workbenchReducer(state, {
      type: "stream_event",
      seq: 6,
      event: toWorkbenchStreamEvent(segment.events[5]!.event),
    });
    expect(replayed).toBe(state);
    expect(replayed.runUsage.total_tokens).toBe(26);
  });

  it("never substitutes zero-usage evidence onto a restored earlier draft", () => {
    const first = completedSegment(1, "job-draft", "run-draft", "Draft?", "draft answer");
    first.events = [
      first.events[0]!,
      { seq: 2, event: { type: "llm_chunk", delta: "draft answer" } },
      {
        seq: 3,
        event: {
          type: "llm_message",
          full: "draft answer",
          usage: { prompt_tokens: 5, completion_tokens: 4, total_tokens: 9 },
        },
      },
    ];
    first.observed_through_seq = 3;
    first.last_event_seq = 3;

    const state = projectProductTranscript({
      product_session_id: "product-session",
      workspace_id: "workspace",
      status: "complete",
      partial_reasons: [],
      segments: [first, completedSegment(2, "job-final", "run-final", "Final?", "final answer")],
    });

    const draft = state.messages.find(
      (message) => message.role === "assistant" && message.content === "draft answer",
    );
    expect(draft?.status).toBe("final");
    expect(draft?.usage).toEqual({
      prompt_tokens: 5,
      completion_tokens: 4,
      total_tokens: 9,
    });
    expect(draft?.promptBuild).toBeUndefined();
    expect(draft?.promptCompaction).toBeUndefined();

    // The second segment starts with the run-scoped usage reset to zero; a
    // duplicated completion for the already-final draft must not stamp those
    // zeros onto it.
    const duplicated = workbenchReducer(state, {
      type: "stream_event",
      seq: 3,
      event: toWorkbenchStreamEvent(first.events[2]!.event),
    });
    const duplicateDraft = duplicated.messages.find(
      (message) => message.role === "assistant" && message.content === "draft answer",
    );
    expect(duplicateDraft).toBe(draft);
    expect(duplicated.messages.filter((message) => message.content === "draft answer")).toHaveLength(1);
    // Replay dedup rejects the already-seen (run, seq) pair: no double count.
    expect(duplicated.runUsage).toBe(state.runUsage);

    const finalAnswer = state.messages.find(
      (message) => message.role === "assistant" && message.content === "final answer",
    );
    expect(finalAnswer?.usage).toEqual({
      prompt_tokens: 1,
      completion_tokens: 1,
      total_tokens: 2,
    });
    expect(duplicated.messages.filter((message) => message.content === "final answer")).toHaveLength(1);
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
          inherited: false,
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
    inherited: false,
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
    inherited: false,
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
