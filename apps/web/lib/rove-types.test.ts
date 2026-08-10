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
    type: "agent_profile_activated",
    identity: {
      selector: { source: "builtin", agent_id: "legacy" },
      agent_id: "legacy",
      display_name: "Legacy Agent",
      definition_version: "compat",
      manifest_hash: "sha256:" + "a".repeat(64),
      package_hash: "sha256:" + "b".repeat(64),
      profile_hash: "sha256:" + "c".repeat(64),
    },
    resumed_from_snapshot: false,
  },
  {
    type: "workspace_instructions_resolved",
    bundle_hash: "sha256:" + "d".repeat(64),
    layer_count: 1,
    rejected_count: 0,
    truncated: false,
  },
  {
    type: "execution_strategy_selected",
    policy: {
      version: 1,
      strategy: "plan_react",
      selection_source: "max_steps_and_plan_flag",
      budgets: { max_step_attempts: 20, max_model_turns_per_step: 4 },
      evaluator_mode: "rule_first_model_on_ambiguity",
      finalizer_policy: "deterministic",
    },
  },
  {
    type: "instruction_overlay_applied",
    target_path: "apps/web/page.tsx",
    scope: "apps/web",
    source_path: "apps/web/AGENTS.md",
    content_hash: "sha256:" + "f".repeat(64),
    boundary: "tool_call",
    call_id: "01JOVERLAY",
  },
  {
    type: "procedures_selected",
    profile_hash: "sha256:" + "c".repeat(64),
    selected: [],
    considered_count: 0,
    excluded_count: 0,
  },
  {
    type: "procedure_hydrated",
    reference: {
      id: "inspect.disk",
      version: "1.0.0",
      trust: "workspace_trusted",
      source_path: "procedures/inspect.disk.md",
      content_hash: "sha256:" + "e".repeat(64),
    },
    truncated: false,
    dropped_bytes: 0,
  },
  {
    type: "execution_budget_updated",
    phase: "step",
    snapshot: {
      limits: { max_step_attempts: 20, max_model_turns_per_step: 4 },
      consumed: {
        plan_steps: 1,
        step_attempts: 1,
        model_turns: 2,
        tool_calls: 1,
        plan_revisions: 0,
        model_repairs: 0,
        planner_turns: 1,
        evaluator_turns: 0,
        replanner_turns: 0,
        finalization_turns: 0,
        wall_time_ms: 1200,
        total_tokens: 640,
        cost_microunits: 0,
      },
      cost_enforced: false,
    },
  },
  {
    type: "execution_degraded",
    record: {
      degradation_id: "deg-1",
      phase: "evaluator",
      code: "evaluator_model_fallback",
      safe_summary: "Deterministic rules were used because the evaluator failed.",
      occurred_at: "2026-08-08T00:00:00Z",
    },
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
    name: "write_file",
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
    type: "tool_artifact_stored",
    call_id: "call-1",
    artifact: {
      artifact_id: "art_0123456789abcdef0123456789abcdef",
      kind: "image",
      mime_type: "image/png",
      byte_length: 2048,
      sha256: "a".repeat(64),
      storage_ref:
        "artifacts/art_0123456789abcdef0123456789abcdef/payload",
      source: {
        run_id: "run-1",
        call_id: "call-1",
        block_ordinal: 0,
        captured_at: "2026-08-09T00:00:00Z",
      },
    },
  },
  {
    type: "tool_artifact_rejected",
    call_id: "call-1",
    block_ordinal: 2,
    reason: "artifact_single_bytes_exceeded",
    observed_bytes: 9000000,
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
        model_repairs: 0,
        planner_turns: 0,
        evaluator_turns: 0,
        replanner_turns: 0,
        finalization_turns: 0,
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
        model_repairs: 0,
        planner_turns: 0,
        evaluator_turns: 0,
        replanner_turns: 0,
        finalization_turns: 0,
        wall_time_ms: 0,
        total_tokens: 0,
        cost_microunits: 0,
      },
    },
  },
  {
    type: "finalization_started",
    record: {
      finalization_id: "fin-1",
      phase: "started",
      finish_reason: "completed",
      mode: "deterministic",
      started_at: "2026-08-08T00:00:01Z",
    },
  },
  {
    type: "finalization_completed",
    record: {
      finalization_id: "fin-1",
      phase: "completed",
      finish_reason: "completed",
      outcome: "success",
      mode: "deterministic",
      started_at: "2026-08-08T00:00:01Z",
      completed_at: "2026-08-08T00:00:02Z",
      output: "Goal: ship\noutcome: success",
      evidence_refs: ["tool_call:call-1"],
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
  {
    type: "steer_accepted",
    id: "steer-1",
    content: "Prioritize the release notes.",
  },
  {
    type: "steer_applied",
    id: "steer-1",
  },
  {
    type: "steer_dropped",
    id: "steer-2",
    reason: "run completed before the steer reached a model turn",
  },
  {
    type: "followup_queued",
    id: "followup-1",
    content: "Draft the release notes next.",
  },
  {
    type: "followup_dequeued",
    id: "followup-1",
  },
  {
    type: "followup_abandoned",
    id: "followup-2",
    reason: "run cancelled",
  },
];

const providerProfileFixtures = [
  { provider_type: "openai", api_base: "https://api.openai.com/v1" },
  { provider_type: "openai-responses", api_base: "https://api.openai.com/v1" },
  { provider_type: "anthropic", api_base: "https://api.anthropic.com" },
  { provider_type: "ollama", api_base: "http://localhost:11434" },
  { provider_type: "fake", api_base: "local" },
] satisfies ProviderProfile[];

describe("rove stream event types", () => {
  it("lists every current runtime event name", () => {
    expect(STREAM_EVENT_NAMES).toEqual(streamEventFixtures.map((event) => event.type));
  });

  it("keeps web provider profiles aligned with the API provider surface", () => {
    expect(
      providerProfileFixtures.map((profile) => profile.provider_type),
    ).toEqual([
      "openai",
      "openai-responses",
      "anthropic",
      "ollama",
      "fake",
    ]);
  });
});
