# Agent Execution Lifecycle StepRunner Plan - 2026-07-20

> Status: **Implemented and verified**
>
> Baseline: Phase 1 commit `4a9cc70`
>
> Design source: [`../design/2026-07-14-agent-execution-lifecycle-design.md`](../design/2026-07-14-agent-execution-lifecycle-design.md)

> Current-state note (2026-07-25): this is a chronological phase ledger. The
> repository later moved into the modular Workspace and removed built-in RAG and
> `rove-index`; the gate commands below are retained as phase-time evidence, not
> current instructions. Use `AGENTS.md` and `docs/runtime/**` for current paths
> and verification.

> Follow-up: the append-only step ledger and canonical `step_result` event were
> implemented in the next phase. The ledger items below remain accurate as
> non-goals of this StepRunner-only phase. See
> [`2026-07-20-agent-execution-lifecycle-step-record-ledger.md`](2026-07-20-agent-execution-lifecycle-step-record-ledger.md).

## Objective

Implement the second dependency-ordered lifecycle phase: a bounded ReAct loop
inside each existing plan step. A successful tool call must return its result
to the model, and only a model step conclusion may complete the step.

This phase deliberately keeps the existing planner, plan cursor, canonical
events, task-state schema, approval path, and replacement-plan compatibility
behavior. It does not pull the later ledger, evaluator, revision, finalizer, or
interface schema phases forward.

## Implemented Contract

```text
plan step start
  -> build focused context
  -> model turn
  -> tool call or batch
  -> existing safety / approval / input / hooks / execution
  -> append assistant call and tool result
  -> model turn in the same step
  -> Action::Final step conclusion
  -> plan step complete
```

The current-step assistant and tool messages are held as bounded step-local
history while the attempt runs. They are injected as prompt prefix material,
which prevents a zero global history window or compaction from dropping the
tool result before the next model turn reads it. Terminal step history is then
merged back into the existing global history for checkpoints and resume.

## Compatibility Budget

The legacy mapping remains one-to-one:

| Existing field | `react` unit | `plan_react` unit |
|---|---|---|
| `runtime.max_steps` | `max_model_turns` | `max_step_attempts` |

Planned execution separately resolves
`DEFAULT_MAX_MODEL_TURNS_PER_STEP = 4`. This is a named compatibility default,
not a second interpretation of `runtime.max_steps`, and it introduces no new
config, CLI, API, persisted-state, or Web field.

Until structured budget events are implemented, exhaustion uses the existing
contract:

- emit `PlanStepFailed` with
  `step model-turn budget exhausted (max_model_turns_per_step=4)`;
- finish with `TerminationReason::StepLimit` and the same explicit reason;
- do not emit `PlanStepCompleted`.

## Failure Semantics

| Condition | Current behavior |
|---|---|
| Successful tool call/batch | Append result and continue the same step |
| Recoverable tool error | Append error result and let the model repair or choose an alternative within the step budget |
| Approval rejection / permission denial | Fail closed; do not replan into another attempt at the same denied mutation |
| Malformed action | Append bounded repair guidance and retry within the step budget |
| Model/internal turn failure | Return a terminal failed step outcome and retain the existing replacement-plan path |
| Hard context limit | Finish with `TokenLimit` |
| Cancellation | Stop new work and finish with `Cancelled` |
| Step model-turn exhaustion | Emit failed step and finish with explicit `StepLimit` reason |

## Files And Boundaries

- `src/core/step_runner.rs`: bounded within-step orchestration and scoped
  history.
- `src/core/plan_loop.rs`: plan drafting/resume, step cursor, lifecycle events,
  and compatibility replanning.
- `src/core/execution.rs`: independent step model-turn compatibility limit.
- `src/core/engine.rs` and `src/core/run_loop.rs`: resolve and pass the limit.
- `src/models/fake.rs`: deterministic static fake responses conclude one
  successful, not-yet-concluded tool result instead of repeating a raw tool
  request forever. Failed and already-concluded tool messages do not trigger
  this behavior.
- `tests/e2e.rs`: tool-result prompt round trip, event ordering, bounded
  exhaustion, same-step recovery, model-failure replanning, resume, and
  persistence coverage.

Provider normalization, `ToolRegistry`, `Executor`, approval/input providers,
hooks, workspace bounds, trace/report/task-state writers, and public event
types are intentionally unchanged.

## Non-Goals

- No append-only `StepRecord` event or persisted ledger.
- No immutable `PlanRevision` chain or typed replacement event.
- No Evaluator, rule-first plan decision, or model-based replan policy.
- No independent Finalizer or deterministic ledger formatter.
- No global model/tool/token/cost accounting or public multidimensional budget
  controls.
- No mid-step resume reconstruction or replay of unknown in-flight side
  effects.
- No CLI/API/Web execution strategy or budget schema changes.

## Verification

Focused behavior:

```powershell
cargo test --test e2e planned_step_
cargo test --test e2e planner_
cargo test --test api api_planned_tool_step_completes_after_successful_tool_call -- --exact
cargo test --test api
```

Repository gates:

```powershell
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo check --features rag --bin rove-index
git diff --check
```

## Definition Of Done

- [x] Tool success cannot directly complete a planned step.
- [x] The next model turn receives the tool result, including with a zero
  global history window.
- [x] `Action::Final` is treated as a step conclusion rather than immediate
  run completion.
- [x] Recoverable tool errors can be repaired within the same bounded step.
- [x] Approval denial and cancellation remain fail closed.
- [x] Step model-turn exhaustion is explicit and cannot emit a completed step.
- [x] Existing plan resume and replacement-plan persistence remain covered.
- [x] Full default Rust and RAG compile gates pass.
- [x] Current runtime documentation agrees with the implemented boundary.
