# Runtime Loop: Plan Outside, ReAct Inside

rove uses a Plan + ReAct runtime shape.

The reusable in-memory mechanics begin in `rove-core`: `core/src/agent.rs`
owns the embeddable Agent loop, `core/src/model_turn.rs` converts normalized
`ModelEvent` values into `AgentEvent` plus `Action`, and
`core/src/parser.rs` owns the compatibility JSON action parser.

`rove-runtime` owns the durable execution surface: IDs, resumable
task/checkpoint and execution-policy data, Workspace/path safety,
prompt/runtime identity, approval/input contracts, canonical `StreamEvent`,
state/trace/artifact/SQLite/repair/resume, context/compaction,
session/durable memory, local tools/MCP, the tool `Executor` and hooks,
runtime-specific tool turns, planning/step coordination, durable event
translation, and the persistent `Engine` facade.
`runtime/src/engine/model_turn.rs` is the synchronous translator from in-memory
`AgentEvent` values into durable `StreamEvent` values. The product default
entry is `runtime::Engine` via `apps/bootstrap::build_engine`; `core::Agent`
is embed-only. Product tool-registry assembly and first-party `AppConfig`
live in product bootstrap and app shells. Runtime tool turns
consume the `rove-core` Tool contract and registry without placing
Workspace, Memory, approval, or input fields on the minimal core `ToolContext`.

The unplanned loop in `runtime/src/engine/run_loop.rs` is the pure ReAct loop
implemented by `run_unplanned_loop`:

1. Build context with `ContextManager::build_with_checkpoint`.
2. Compact old history when the token budget requires it.
3. Run one model turn through the `rove-core` `run_model_turn` adapter.
4. Normalize native OpenAI, Anthropic, Ollama, and Responses tool-use into `Action`.
5. Run one tool turn through `run_tool_turn`.
6. Append assistant tool calls and tool results back into history.
7. Repeat until final answer, cancellation, token limit, step limit, or error.

The planned coordinator, `run_planned_loop` in `runtime/src/engine/plan_loop.rs`,
delegates each current plan step to the bounded runner in
`runtime/src/engine/step_runner.rs`:

1. Draft or resume a `TaskPlan`.
2. Convert the current plan step into a focused user prompt.
3. Build context with prior global history plus a step-local message prefix.
4. Run a model turn through the shared `run_model_turn` helper.
5. Execute a tool call or batch through the existing safety, approval, input,
   hook, and `run_tool_turn` path.
6. Append the assistant tool call and tool result to step-local history, then
   return that result to the model within the same plan step.
7. Keep recoverable tool errors in the same bounded step so the model can
   correct arguments or choose a safe alternative. Approval rejection remains
   fail closed and does not enter the replacement-plan path.
8. Mark the step complete only after `Action::Final` supplies the step
   conclusion. A successful tool call does not complete the step by itself.
9. Evaluate every terminal `StepRecord` with the deterministic rule-first
   evaluator. Continue, finish with a typed reason, or replace only the
   remaining work after an explicitly recoverable failure.

Step-local messages are included as prompt prefix material rather than ordinary
trimmable global history. This guarantees that the next model turn receives the
current tool result even when the configured global history window is zero or
older history is being compacted. When compaction changes the summary, the
runner rebuilds the actual model prompt before issuing the turn.

The engine now resolves the legacy `plan_enabled` and `max_steps` fields through
the typed `runtime::execution::ExecutionPolicy` boundary before selecting a loop.
Those CLI/API fields are sugar that write into `ExecutionPolicy`; the policy is
the sole execution-config truth.
`react` maps the old limit to `max_model_turns`, while `plan_react` maps it only
to `max_step_attempts`. Planned execution separately resolves
`max_model_turns_per_step = 4` as a named compatibility default; it does not
reinterpret `max_steps` as a second budget unit. Exhausting this step-local
ceiling emits a terminal `step_result` and completes the run with
`TerminationReason::StepLimit` plus an explicit
`max_model_turns_per_step=4` reason.

Planned execution also maintains an append-only terminal-attempt ledger:

1. `PlanCreated` includes a stable logical `plan_id`, a
   `plan_revision_id`, a monotonic compatibility revision number, and the full
   immutable initial `PlanRevision`.
2. `PlanStepStarted` includes the matching plan/revision identity, stable step
   ID, attempt number, and start time.
3. The bounded runner derives model-turn, tool-call, mutation, and token usage
   from the canonical model/tool events emitted during that attempt.
4. Every terminal attempt emits `step_result` with a `StepRecord`, followed by
   exactly one `plan_decision`. Compatibility dual-fire
   `PlanStepCompleted` / `PlanStepFailed` events are not emitted. The runtime
   currently produces `succeeded`, `failed`, `blocked`, `budget_exhausted`,
   `cancelled`, and `interrupted` records.
5. A recoverable failure emits `plan_revised` with an immutable child revision
   linked to its parent revision, triggering step record, and decision. It does
   not masquerade as another initial `plan_created` event.
6. `trace.jsonl` stores the canonical append-only events, `task_state.json`
   stores the materialized records/decisions/revisions, `PromptCheckpoint`
   stores bounded lifecycle metadata, and `report.json` includes the full
   lifecycle projections. SQLite indexes the same event stream and remains
   rebuildable through `rove state repair`.

Resume consumes the materialized ledger conservatively. A persisted terminal
record missing its decision is evaluated exactly once before execution
continues. A persisted successful terminal record advances the saved plan
without replaying the step. A complete in-flight attempt with no terminal
record is closed as `interrupted`, followed by a typed finish decision and an
error completion; model and tool execution are not restarted automatically
because an external side effect may already have occurred. Older snapshots
that contain only a mutable plan are wrapped once as revision zero with the
`legacy_plan_migrated` reason code.

The current evaluator is deterministic and provider-free: it maps terminal
status plus explicit recoverability to `continue`, `replace_remaining`, or
`finish`. Model-on-ambiguity evaluation, an independent Finalizer, public
multidimensional budget configuration, global model/tool/token accounting,
structured budget events, and reconciliation of canonical trace events newer
than the latest `TaskState` snapshot remain future work.

This differs from pico's `pico/agent_loop.py`, where prompt build, model call,
parse, tool execution, checkpoint, and trace recording live in one readable loop.
rove keeps those phases split into focused Rust modules so provider streaming,
cancellation, artifact persistence, API jobs, and plan recovery can evolve
independently.

The conceptual ReAct unit in rove is:

```text
ReactTurn =
  ContextBuild
  -> ModelTurn
  -> Action
  -> ToolTurn
  -> HistoryAppend
```

For a planned step, that unit repeats until a step conclusion or a terminal
step outcome:

```text
StepRunner =
  bounded ReactTurn*
  -> Action::Final as step conclusion
  -> StepResult
  -> PlanDecision
  -> Continue / PlanRevised / Finish
```

`Engine` is the orchestration shell. It loads resume state and memory, chooses
planned or unplanned mode, streams events, and writes run artifacts.
