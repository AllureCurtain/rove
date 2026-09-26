# Runtime Loop: Plan Outside, ReAct Inside

rove uses a Plan + ReAct runtime shape.

The reusable execution mechanics begin in `rove-core`:
`core/src/kernel.rs` owns the callback-driven, Runtime-neutral multi-turn Agent
kernel; `core/src/agent.rs` supplies its in-memory embedding host;
`core/src/model_turn.rs` converts normalized `ModelEvent` values into
`AgentEvent` plus `Action`; and `core/src/parser.rs` owns the compatibility JSON
action parser.

Structured provider tool calls are authoritative. `ModelClient` exposes an
explicit `compatibility_text_tool_calls()` opt-in, and only clients that require
it (currently the `fake-raw` test profile) parse JSON text actions. Native
OpenAI Completions/Responses, Anthropic, Ollama, and the default Fake client do
not reinterpret ordinary assistant text as a tool call. Malformed opted-in
payloads become typed `Action::Malformed` recoverable failures; they cannot be
accepted as a terminal answer. Executor schema diagnostics are deterministic
and identify the field, expected/received JSON types and value, and a bounded
correction example.

`rove-runtime` owns the durable execution surface: IDs, resumable
task/checkpoint and execution-policy data, Workspace/path safety,
prompt/runtime identity, approval/input contracts, canonical `StreamEvent`,
state/trace/artifact/SQLite/repair/resume, context/compaction,
session/durable memory, local tools/MCP, the tool `Executor` and hooks,
Runtime kernel hosts and tool turns, planning/step coordination, durable event
translation, and the persistent `Engine` facade.
Before either host starts, Engine compiles the qualified Agent selector into an
immutable run profile. Its capability set filters model-visible tool schemas and
is rechecked before dispatch; its root instructions, selected procedures, and
bounded hydrated bodies enter working context without becoming permissions.
Nested `AGENTS.md` layers are added only for matching paths. A first call into a
new nested scope is closed with `precondition_required` before dispatch, then the
next turn receives that scoped layer and may retry through the normal safety
path.
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

Every model turn also receives bounded runtime facts: provider mode, workspace
and capability state, available tool names, active approval policy, execution
bounds, and typed recovery behavior. Prompt component byte counts, profile
identity, stable prefix/tool signatures, and cache identity are content
addressed and remain stable for stable inputs. Restricted workspaces are
tested to ensure project instructions and procedures do not activate without
their independent trust capabilities.

Tool history keeps the current tool round inline. Older repeated artifact-backed
results project to deterministic `RichReference` blocks while the canonical
`ToolArtifactStore` retains content, provenance, quota, MIME, sensitivity, and
retention facts. Planned-step in-flight history is supplied through the required
history path so it is not discarded by stable-prefix compaction. After resume or
cleanup, `resolve_tool_artifact` resolves bounded UTF-8 ranges through the
canonical authority and returns typed malformed, missing/expired, sensitive,
non-text, or invalid-boundary failures. Provider history projection preserves
tool-call/result pairing for every supported wire protocol.

The engine now resolves the legacy `plan_enabled` and `max_steps` fields through
the typed `runtime::execution::ExecutionPolicy` boundary before selecting a loop.
Those CLI/API fields are sugar that write into `ExecutionPolicy`; the policy is
the sole execution-config truth.
`react` maps the old limit to `max_model_turns`, while `plan_react` maps it only
to `max_step_attempts`. Planned execution separately resolves
`max_model_turns_per_step = 4` as a named compatibility default; it does not
reinterpret `max_steps` as a second budget unit. Exhausting this step-local
ceiling emits a terminal `step_result` whose `error_code` names the exhausted
dimension (`model_turns_per_step_budget_exhausted`) and completes the run with
`TerminationReason::StepLimit`.

Operators can configure the remaining dimensions directly under
`[runtime.execution]` in project configuration. Every field is optional: an unset
dimension keeps the value the `max_steps` projection derived, so an existing
config behaves exactly as before. Configured values overlay the derived policy
and the resolved result is validated at startup, so a zero limit or a per-step
ceiling above its global ceiling fails as a configuration error rather than a
mid-run refusal. Available keys are `evaluator_mode`, `finalizer_policy`,
`max_plan_steps`, `max_step_attempts`, `max_model_turns`,
`max_model_turns_per_step`, `max_tool_calls`, `max_tool_calls_per_step`,
`max_plan_revisions`, `max_model_repairs`, `max_finalization_turns`,
`max_wall_time_ms`, `max_total_tokens`, and `max_cost_microunits`. Cost is
enforced only when the active provider supplies priced usage; the
`cost_enforced` flag on a budget snapshot reports whether that is true rather
than implying enforcement that does not exist.

Budgets are per-run accounting. Restarting the *same* run restores its consumed
usage so a crash-restart loop cannot hand out a fresh allowance on every
attempt, while a new turn that continues a session starts from zero. Carrying
usage across turns would progressively starve a long session until no further
work could run.

Model-call retries are configured separately from execution budgets, under
`[runtime.recovery.retry]`. Every field is optional and an unset field keeps the
runtime default. The default budget is **enabled**, so adding the section does
not change parsing but an unconfigured runtime retries where it previously did
not; set both attempt fields to `1` to restore the pre-retry behaviour.

```toml
[runtime.recovery.retry]
rate_limit_max_attempts = 6   # total attempts for a throttled call, first included
transient_max_attempts = 4    # total attempts for a transport failure
backoff_base_ms = 2000        # first wait; doubles per retry
backoff_max_ms = 30000        # ceiling, and the clamp for a provider retry-after
```

`1` in both attempt fields disables retry for that class; `0` is clamped to `1`
rather than becoming a second way to express "never". A configured base above the
ceiling keeps the ceiling authoritative. Jitter is intentionally not
configurable: it only spreads retries out. See "Model-call retry budget" below
for the retry contract itself.

`ExecutionStrategySelected`, `ExecutionBudgetUpdated`, and `ExecutionDegraded`
are canonical events. A degradation record is always explicit: a fallback never
changes permissions, never erases recorded evidence, and carries a safe summary
rather than model reasoning.

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
   currently produces `succeeded`, `failed`, `blocked`, `rejected`,
   `budget_exhausted`, `cancelled`, `interrupted`, and `indeterminate` records.
   A tool dispatch that stopped without a recorded outcome is classified
   `indeterminate` rather than treated as safely replayable.
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

The evaluator remains rule-first: deterministic rules map terminal status plus
explicit recoverability to `continue`, `replace_remaining`, or `finish`. A model
evaluation is reachable only when a terminal record carries a validated
`PlanAmbiguity` produced from a structured step conclusion; arbitrary prose never
grants access to the evaluator. Model evaluation is bounded by repair and
model-turn budgets, rejects a no-op replan, and falls back to the deterministic
decision on any error, invalid result, cancellation, or budget boundary. Set
`evaluator_mode = "rule_only"` to disable it entirely.

An independent Finalizer owns the user-facing answer. It is evidence-grounded:
it synthesizes from recorded step facts and cites the evidence that produced
them, and it never labels a non-success terminal state as completed. Every
outcome is explained — `success`, `partial`, `blocked`, `rejected`, `cancelled`,
`interrupted`, `exhausted`, `indeterminate`, and `failed` — so a cancelled or
exhausted run reports what happened instead of returning no answer. React finals
stay direct because the model already produced the user-facing answer; planned
runs use the deterministic synthesis by default. `finalizer_policy =
"model_preferred"` prefers a bounded model synthesis and falls back
deterministically, emitting an `ExecutionDegraded` record when it does.

Resume reconciles the canonical trace tail with the snapshot. Because
`task_state.json` is written after the `trace.jsonl` line, a crash between those
writes leaves durable facts only in the trace. On resume, events newer than
`checkpoint.last_event_seq` are replayed into the snapshot as a projection: no
tool call, model turn, or mutation is re-dispatched, application is idempotent,
an exhaustion boundary and a resolved finalization outcome are sticky, only a
successful record advances plan progress, and a truncated tail line is counted
and skipped rather than failing the resume.

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
provider capability checks, Runtime-owned capability snapshot binding, the single
shared Agent kernel, and the independent lifecycle Finalizer are implemented.

## Model-call retry budget

A model call that fails before producing any output is retried by the run loop,
not by the provider or routing layer. The budget lives in
`runtime/src/engine/recovery.rs` (`ProviderRetryPolicy`) and is spent inside
`run_kernel_model_turn`, so it covers the React host and the planned-step host
alike and also covers a directly assembled provider that
`RoutingModelClient` never wraps. Those two hosts are the covered call sites:
the Planner, Replanner, Evaluator, Finalizer, and model compaction call
`ModelClient::stream` directly and are not retried by this budget.

- **Independent budgets.** Rate limiting and transient failures
  (`RequestFailed`, `StreamInterrupted`) each have their own attempt budget:
  six total attempts for throttling, four for transport failures, first call
  included. A throttled provider therefore cannot consume the transport budget.
  `max_attempts = 1` disables retry for that class, and both at `1` restore the
  pre-budget behavior exactly.
- **Backoff.** The first retry waits `backoff_base_ms` (default 2s) and each
  further retry doubles it, capped at `backoff_max_ms` (default 30s) with a
  small jitter that only shaves the delay. A provider `RateLimited { retry_after_ms }`
  is used as given, clamped to the ceiling and never jittered: the provider
  asked for that wait.
- **Visibility.** Every scheduled retry emits
  `StreamEvent::ProviderRetry { attempt, max_attempts, delay_ms, reason, phase }`
  before sleeping. `attempt` is 1-based, so the first retry is `attempt = 2`, and
  `reason` is a whitelist (`rate_limited` or `transient:<error_code>`). The event
  carries no provider text: messages, headers, and response bodies never reach
  it. (A terminal `ModelError` still reaches the run-completion output and
  therefore `trace.jsonl` and `report.json`, exactly as it did before this
  budget existed; bounding that text is a separate open gap, not something the
  retry event changes.) CLI/TUI and the Web project these facts; they do not
  invent a policy.
- **Cancellation.** The wait is cancellable and biased: cancelling during a
  backoff ends the turn immediately as cancelled instead of leaving a pending
  sleep or issuing one more request.
- **Budget accounting.** A retry is the same model call, so it does not consume
  `budgets.max_model_turns` and does not advance a plan step attempt. The bound
  is the retry budget itself: at the defaults one model call can issue nine
  provider calls (the first call plus five rate-limit and three transient
  retries) and sleep roughly 74 seconds in total backoff. Backoff is not charged
  against `max_wall_time_ms`; that dimension is evaluated on the planned-execution
  budget path, and the React path does not refresh it. An accepted steer is
  announced once per model call, not once per attempt.
- **Independent counters.** `attempt` and `max_attempts` are per class, so a
  sequence can read 5/6 and then 2/4. A single monotonic counter across both
  classes would be a display change, not a budget change.
- **No retry after output.** A retry is only allowed while the turn has not yet
  produced a `LlmChunk`. Once text has streamed, a failure is terminal for the
  turn: regenerating would duplicate text the user can already see. The same
  rule means a partly streamed tool call is safe to retry, because no tool has
  been dispatched yet, and that a Review-mode turn (whose text is redacted and
  never becomes `LlmChunk`) stays retryable.
- **Exhaustion.** When the budget for the failure's class is spent, the
  provider's own `ModelError` reaches the kernel unchanged and the run
  terminates as before (`TerminationReason::Error`); a recoverable planned-step
  failure then still enters the existing repair/replan path.

## Cancellation and abort salvage

Cancelling a run ends it as `Cancelled`. What happens to the model turn that was
in flight at that moment is the abort-salvage contract (runtime document R2b),
implemented in `runtime/src/engine/model_turn.rs` and shared by the React host
and the planned-step host because both call the same `run_kernel_model_turn`.

- **The window is a runtime decision.** `ABORT_SALVAGE_WINDOW` is a private
  constant of 1500 ms. It is not configuration: it bounds how long a stop may
  take to become terminal, which is a contract, not a deployment preference.
  `rove-core` knows no window at all — a cancelled turn only hands back its
  in-memory outcome, `CancelledTurn { partial, usage }`, where `partial` is the
  text it accumulated and `usage` is what the provider actually reported (or the
  type's default when it reported none).
- **Only text starts a window.** The entry condition is text the turn already
  published as `LlmChunk` — the same "has produced output" fact the retry budget
  uses. A stop with no published text ends the turn immediately and persists
  nothing, which is what the Web smart-stop branch reads as "the model produced
  nothing". A Review-mode turn never publishes `LlmChunk`, so it never salvages.
- **Two outcomes, one terminal state.** If the in-flight request finishes inside
  the window with a complete message, that message is persisted with
  `aborted = false`. If the window expires, the accumulated text is persisted
  with `aborted = true` and the request is cancelled. The run ends as
  `Cancelled` either way — "the user stopped this" and "this text is complete"
  are independent facts and neither overwrites the other.
- **One event writes both surfaces.** The persisted result is a single canonical
  `StreamEvent::LlmMessage { full, usage, tool_calls: [], assistant_turn, aborted }`.
  The recorder projects it into resumable history (`task_state.json`) and into
  the trace, and API SSE / Web consume the same event, so no private channel and
  no half-written state exists. Deltas that arrive inside the window are counted
  but not republished: the stream was already stopped for every consumer.
- **The marker is additive.** `aborted` is a `#[serde(default)]` boolean on
  `LlmMessage`, so older traces and older clients read it as `false` (complete),
  the variant name and `llm_message` event name are unchanged, and the OpenAPI
  projection is untouched because `JobStreamEvent.event` is an opaque object.
  The resumable-history projection drops the marker on purpose: it is a
  presentation fact, not recoverable state. Review-mode persistence passes it
  through instead, so redaction cannot silently drop a field.
- **Text only, tools untouched.** Salvage never dispatches a tool, never starts a
  model call, and never changes an approval. A cancel during a tool call keeps
  its existing behavior (the tool turn's biased cancel select ends it before or
  during execution exactly as before). A salvaged message is text-only with
  `stop_reason = Cancelled` and no tool calls, so resume cannot discover an
  executable tool from it.
- **Idempotent and resumable.** A second cancel cannot reopen the window or write
  a second message or event. The salvaged text is ordinary assistant history: on
  resume it is visible once and is never replayed.
- **Embedded hosts.** `rove-core`'s in-memory Agent loop has no durable history to
  salvage into, so it reports the cancelled outcome and persists nothing. The
  window lives with persistence, in the runtime.
- **Clients.** The Web marks such a message with an `(aborted)` / `(已中止)`
  suffix beside its text and restores the marker with the run's canonical events.
  The smart-stop notice branch stays reserved and unused (see the frontend
  document §7.5).
