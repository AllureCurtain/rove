import { describe, expect, it } from "vitest";

import { STREAM_EVENT_NAMES, type ProviderProfile, type StreamEvent } from "./rove-types";

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
    plan_id: "plan-1",
    plan_revision_id: "revision-0",
    revision: 0,
    plan_revision: {
      plan_id: "plan-1",
      revision_id: "revision-0",
      revision: 0,
      created_at: "2026-07-20T00:00:00Z",
      decision_id: "initial-decision",
      safe_reason_codes: ["planner_draft"],
      remaining_steps: [{ id: "1", title: "Inspect", done: false }],
      budget_snapshot: {
        plan_steps: 0,
        step_attempts: 0,
        model_turns: 0,
        tool_calls: 0,
        plan_revisions: 0,
        wall_time_ms: 0,
        total_tokens: 0,
        cost_microunits: 0,
      },
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
    type: "step_result",
    record: {
      record_id: "record-1",
      plan_id: "plan-1",
      plan_revision_id: "revision-0",
      step_id: "1",
      attempt: 1,
      status: "failed",
      started_at: "2026-07-20T00:00:00Z",
      finished_at: "2026-07-20T00:00:01Z",
      summary: "Step failed and may be replanned.",
      completion_basis: "runtime_failure",
      model_turns_used: 1,
      tool_calls_used: 1,
      token_usage: {
        prompt_tokens: 1,
        completion_tokens: 2,
        total_tokens: 3,
      },
      error_code: "step_runtime_failure",
      safe_error_summary: "The planned step ended with a runtime failure.",
    },
  },
  {
    type: "plan_decision",
    record: {
      trigger_step_record_id: "record-1",
      decided_at: "2026-07-20T00:00:02Z",
      decision: {
        decision_id: "decision-1",
        kind: "replace_remaining",
        safe_reason_codes: ["recoverable_step_failure"],
        safe_summary: "Replace the failed remaining work.",
        remaining_work_requirements: ["Use a safe alternative."],
      },
    },
  },
  {
    type: "plan_revised",
    plan: {
      goal: "test",
      current_step: 0,
      steps: [{ id: "2", title: "Inspect safely", done: false }],
    },
    revision: {
      plan_id: "plan-1",
      revision_id: "revision-1",
      parent_revision_id: "revision-0",
      revision: 1,
      created_at: "2026-07-20T00:00:03Z",
      trigger_step_record_id: "record-1",
      decision_id: "decision-1",
      safe_reason_codes: ["recoverable_step_failure"],
      remaining_steps: [{ id: "2", title: "Inspect safely", done: false }],
      budget_snapshot: {
        plan_steps: 0,
        step_attempts: 0,
        model_turns: 0,
        tool_calls: 0,
        plan_revisions: 0,
        wall_time_ms: 0,
        total_tokens: 0,
        cost_microunits: 0,
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
      source_message_count: 3,
    },
  },
  {
    type: "memory_flushed",
    notes: ["Promoted durable memory before compaction"],
  },
  {
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
  {
    type: "run_completed",
    reason: "final",
    output: "done",
  },
];

const providerProfileFixtures = [
  { channel: "openai", api_base: "https://api.openai.com/v1" },
  { channel: "openai-responses", api_base: "https://api.openai.com/v1" },
  { channel: "anthropic", api_base: "https://api.anthropic.com" },
  { channel: "ollama", api_base: "http://localhost:11434" },
  { channel: "fake", api_base: "local" },
  {
    // Advanced escape hatch still accepted.
    wire_protocol: "openai-chat",
    api_base: "https://gateway.example.test/v1",
  },
] satisfies ProviderProfile[];

describe("rove stream event types", () => {
  it("lists every current runtime event name", () => {
    expect(STREAM_EVENT_NAMES).toEqual(streamEventFixtures.map((event) => event.type));
  });

  it("keeps web provider profiles aligned with the API provider surface", () => {
    expect(
      providerProfileFixtures.map(
        (profile) => profile.channel ?? profile.wire_protocol,
      ),
    ).toEqual([
      "openai",
      "openai-responses",
      "anthropic",
      "ollama",
      "fake",
      "openai-chat",
    ]);
  });
});
