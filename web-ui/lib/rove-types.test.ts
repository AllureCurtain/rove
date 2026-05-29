import { describe, expect, it } from "vitest";

import { STREAM_EVENT_NAMES, type StreamEvent } from "./rove-types";

const streamEventFixtures: StreamEvent[] = [
  {
    type: "run_started",
    run_id: "run-1",
    job_id: "job-1",
    user_message: "hello",
  },
  {
    type: "llm_chunk",
    delta: "he",
  },
  {
    type: "model_status",
    status: "thinking",
    message: "Model is thinking",
  },
  {
    type: "llm_message",
    full: "hello",
    usage: {
      prompt_tokens: 1,
      completion_tokens: 2,
      total_tokens: 3,
    },
    tool_calls: [
      {
        id: "toolu-1",
        name: "echo",
        args: { text: "hello" },
      },
    ],
  },
  {
    type: "tool_call_started",
    call_id: "call-1",
    tool_use_id: "toolu-1",
    name: "echo",
    args: { text: "hello" },
  },
  {
    type: "tool_call_approval_needed",
    call_id: "call-1",
    name: "fs_write",
    args: { path: "notes.md" },
    reason: "destructive tool requires explicit approval",
  },
  {
    type: "tool_call_completed",
    call_id: "call-1",
    result: {
      call_id: "call-1",
      output: "wrote notes.md",
      mutations: [
        {
          path: "notes.md",
          operation: "update",
          diff: "-old\n+new",
        },
      ],
    },
  },
  {
    type: "tool_call_failed",
    call_id: "call-1",
    error: {
      code: "invalid_args",
      reason: "missing path",
    },
  },
  {
    type: "input_needed",
    input_id: "input-1",
    prompt: "Which branch?",
  },
  {
    type: "plan_created",
    plan: {
      goal: "test",
      current_step: 0,
      steps: [{ id: "1", title: "Inspect", done: false }],
    },
  },
  {
    type: "plan_step_started",
    index: 0,
    step: { id: "1", title: "Inspect", done: false },
  },
  {
    type: "plan_step_completed",
    index: 0,
    step: { id: "1", title: "Inspect", done: true },
  },
  {
    type: "plan_step_failed",
    index: 0,
    step: { id: "1", title: "Inspect", done: false },
    reason: "tool failed",
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
      source_message_count: 3,
    },
  },
  {
    type: "run_completed",
    reason: "final",
    output: "done",
  },
];

describe("rove stream event types", () => {
  it("lists every current runtime event name", () => {
    expect(STREAM_EVENT_NAMES).toEqual(streamEventFixtures.map((event) => event.type));
  });
});
