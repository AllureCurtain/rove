# Runtime Loop: Plan Outside, ReAct Inside

rove uses a Plan + ReAct runtime shape.

The reusable execution mechanics begin in `rove-core`:
`core/src/kernel.rs` owns the callback-driven, Runtime-neutral multi-turn Agent
kernel; `core/src/agent.rs` supplies its in-memory embedding host;
`core/src/model_turn.rs` converts normalized `ModelEvent` values into
`AgentEvent` plus `Action`; and `core/src/parser.rs` owns the compatibility JSON
action parser.

`rove-runtime` owns the durable execution surface: IDs, resumable
task/checkpoint and execution-policy data, Workspace/path safety,
prompt/runtime identity, approval/input contracts, canonical `StreamEvent`,
state/trace/artifact/SQLite/repair/resume, context/compaction,
session/durable memory, local tools/MCP, the tool `Executor` and hooks,
Runtime kernel hosts and tool turns, planning/step coordination, durable event
translation, and the persistent `Engine` facade.
`runtime/src/engine/model_turn.rs` is the synchronous translator from in-memory
`AgentEvent` values into durable `StreamEvent` values. The product default
entry is `runtime::Engine` via `apps/bootstrap::build_engine`; `core::Agent`
is embed-only. Product tool-registry assembly and first-party `AppConfig`
live in product bootstrap and app shells. Runtime tool turns
consume the `rove-core` Tool contract and registry without placing
Workspace, Memory, approval, or input fields on the minimal core `ToolContext`.

The `run_unplanned_loop` host in `runtime/src/engine/run_loop.rs` delegates the
ReAct state machine to `rove_core::run_agent_kernel`:

1. Build context with `ContextManager::build_with_checkpoint`.
2. Compact old history when the token budget requires it.
3. Let the Core kernel run one normalized model turn and interpret its `Action`.
4. Normalize native OpenAI, Anthropic, Ollama, and Responses tool-use through
   the existing Core `run_model_turn` boundary.
5. Let the kernel dispatch one Runtime `run_tool_turn` callback, forwarding
   approval/input events while the tool is waiting.
6. Append the host-produced canonical assistant/tool messages to kernel-owned
   history.
7. Let the kernel repeat until final answer, cancellation, token limit, step
   limit, or error.

The planned coordinator, `run_planned_loop` in `runtime/src/engine/plan_loop.rs`,
delegates each current plan step to the bounded runner in
`runtime/src/engine/step_runner.rs`:

1. Build or reuse the Engine-pinned capability snapshot, then draft or resume a
   `TaskPlan`. Planner receives its bounded metadata summary and cannot invoke
   tools.
2. Convert the current plan step into a focused user prompt.
3. Build context with prior global history plus a step-local message prefix in
   the StepRunner kernel host.
4. Run the same Core kernel used by embedded and unplanned execution.
5. Execute each kernel tool action through the existing safety, approval,
   input, hook, and `run_tool_turn` path.
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
   immutable initial `PlanRevision`; the revision pins the current capability
   snapshot ID.
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

`Engine` is the durable orchestration shell. It loads resume state and memory,
chooses planned or unplanned Runtime hosts, streams events, and writes run
artifacts. The Core kernel owns model/action/tool repetition, cancellation and
limits, whole-batch reservation, final/follow-up transitions, and history
progression. Runtime hosts own prompt construction, compaction, durable steer
lifecycle facts, approvals/input, Runtime hooks, tool execution, and canonical
event translation.

## Typed model-turn boundary

The current provider-neutral boundary is implemented in `rove-models` and
`rove-core`:

- `AssistantTurn` carries ordered bounded content, typed tool calls, usage, a
  normalized stop reason, and safe provenance. `InternalCallId` is distinct
  from the provider wire reference.
- `TurnAssembler` is shared by every native and Fake stream after wire
  decoding. It bounds text/argument bytes and call counts, validates start /
  delta / done correlation, and requires a terminal `Done` event before Core
  can create an `Action` for strict clients.
- A truncated stream, duplicate/unknown call, conflicting name, or non-object
  arguments becomes a typed model failure before ToolRegistry policy or tool
  execution. Valid provider argument fragments are completed by the terminal
  call object.

`ProviderClient`, the external adapter, `RoutingModelClient` when all selected
targets are strict, and the shared `FakeModelClient` require `Done`; EOF is a
typed incomplete-stream failure. Existing embedded `ModelClient` implementations
remain dual-compatible through the default legacy EOF marker while they migrate
to `requires_terminal_event()`. Even in that compatibility mode, the shared
assembler still rejects incomplete calls, duplicate or conflicting identities,
invalid arguments, and oversized content before ToolRegistry execution.

The existing `Message` history JSON remains readable. Typed session entries
and `HistoryProjector` are additive; they project canonical call/result pairs
to target-valid wire IDs without mutating persisted history. Legacy tool
results without native IDs are accepted only through the explicit deterministic
compatibility policy. Provider-specific payloads remain inside `rove-models`.

## Durable typed session boundary

The first-wave typed session boundary is connected to the runtime artifact path.
`rove-runtime::Session` uses schema version `1` and stores provider-neutral
`SessionEntry` values for user content, assistant turns, and tool results. A
`PromptCheckpoint` written by the current recorder always includes this
canonical session. `TaskState.history` and `PromptCheckpoint.preserved_tail`
remain serialized compatibility projections for older readers; new code does
not treat either projection as an independent source of truth.

Readers dual-read old `TaskState` and artifact snapshots that contain only
`Vec<Message>`, converting them through the deterministic legacy projection.
Once such a run is snapshotted again, the writer emits the schema-1 canonical
session and derives the compatibility fields from it. Unknown additive fields
are ignored, missing additive fields use defaults, and a future or invalid
session schema is rejected before model execution. Rolling back the binary is
safe for old snapshots because the derived legacy fields are still present; a
newer session schema is fail-closed rather than replayed or silently downgraded.

Resume and provider requests project canonical history through the selected
`ModelClient::history_protocol()`. Internal call IDs, tool names, result status,
and errors stay canonical while target-specific wire IDs are regenerated for
each provider. The complete canonical session remains persisted for audit and
future derivation, but resume never projects that full history into the prompt.
It first closes a trailing in-flight round, then takes a correlation-safe
canonical suffix with a 12-entry target and projects only that suffix beside
the checkpoint summary. The suffix may expand only as needed to keep an
assistant multi-tool round and all of its results together. Old checkpoints
with no canonical `session` continue to use
their already bounded `preserved_tail`. A trailing in-flight round found at
termination or resume is closed once with explicit `unknown_effect` /
`interrupted` results so it cannot be replayed. Orphan, duplicate, non-trailing
missing, or conflicting results cause projection to fail rather than entering
`ToolRegistry`.

The shared kernel consumes the bounded derived `Vec<Message>` view produced by
each host at the existing context-manager boundary. Authoritative bounded
tool-schema validation, registration-time descriptor pinning, pre-dispatch
provider capability checks, Runtime-owned capability snapshot binding, and the
single shared Agent kernel are implemented. An independent lifecycle Finalizer
remains later work in the implementation brief.
