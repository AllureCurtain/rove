# Agent Execution Lifecycle StepRecord Ledger Plan - 2026-07-20

> Status: **Implemented and verified**
>
> Baseline: StepRunner commit `5747a1c`
>
> Design source:
> [`../design/2026-07-14-agent-execution-lifecycle-design.md`](../design/2026-07-14-agent-execution-lifecycle-design.md)

## Objective

Implement the third dependency-ordered lifecycle phase: make every terminal
planned-step attempt an append-only `StepRecord` fact, persist a resumable
ledger projection, and prevent automatic replay of an unknown in-flight side
effect.

This phase builds on the bounded StepRunner. It does not pull forward the full
plan-revision, evaluator, finalizer, or public multidimensional-budget phases.

## Implemented Contract

```text
PlanCreated(plan_id, plan_revision_id, revision)
  -> PlanStepStarted(step identity, attempt, started_at)
  -> bounded model/tool events
  -> step_result(StepRecord)
  -> compatibility PlanStepCompleted / PlanStepFailed
```

The canonical `StepRecord` includes:

- stable record, logical plan, plan revision, step, and attempt identity;
- terminal status, start/finish timestamps, bounded summary, and completion
  basis;
- tool call IDs and resolvable `tool_call:<id>` evidence references;
- deterministic tool mutations reported by completed calls;
- model-turn, tool-call, and token usage for the attempt;
- typed safe error code/summary for non-success outcomes.

The runtime currently produces records for `succeeded`, `failed`, `blocked`,
`budget_exhausted`, `cancelled`, and `interrupted`. The semantic
`partial`/`skipped` values remain available for later lifecycle phases but are
not produced by the current coordinator.

## Storage And Interface Projection

The phase reuses existing durable sources rather than adding another ledger
file or mutable database truth:

| Surface | Contract |
|---|---|
| `trace.jsonl` | Canonical append-only `step_result` event |
| `task_state.json` | Full materialized step records plus optional active attempt |
| `PromptCheckpoint` | Bounded plan/revision identity, record count, and active attempt |
| `report.json` | Final run projection of all terminal step records |
| SQLite | Rebuildable event/index projection through existing trace repair |
| API/SSE | Structured event replay with canonical `step_result` event name |
| Terminal/Web | Deduplicated structured record state; compatibility events retain the visible timeline row |

Old plan events, task states, prompt checkpoints, and reports deserialize with
default empty lifecycle metadata. The existing `TaskPlan` wire shape and
`PlanStepCompleted` / `PlanStepFailed` events remain readable during the
compatibility window.

## Resume Safety

Resume follows conservative side-effect semantics:

- a persisted successful terminal record advances a stale plan cursor without
  replaying the step;
- a persisted terminal failure continues its deterministic compatibility
  transition without rerunning the attempt;
- a complete active attempt with no terminal record is closed by a new
  `interrupted` `StepRecord` and the resumed run terminates with an error;
- unknown model/tool work is never started automatically after that
  interruption.

`StepResult` is handled before the compatibility completion event by the
artifact recorder, so a successful record immediately clears the active
attempt and advances the materialized plan. Cross-artifact reconciliation of a
canonical trace tail newer than the latest task-state snapshot remains a later
recovery phase.

## Files And Boundaries

- `src/core/execution.rs`: plan/attempt identity and materialized ledger types.
- `src/core/events.rs`: lifecycle identity fields and canonical `StepResult`.
- `src/core/step_runner.rs`: event-derived per-attempt metrics.
- `src/core/plan_loop.rs`: terminal record construction, ordering, revision
  identity, and conservative resume transitions.
- `src/core/types.rs` and `src/core/engine.rs`: task/checkpoint and resume
  plumbing.
- `src/state/artifacts.rs`, `src/state/report.rs`, and `src/state/resume.rs`:
  durable projections and backward compatibility.
- `src/interfaces/terminal/view.rs` and `web-ui/lib/rove-*`: structured
  consumer projections without duplicate visible timeline entries.
- `tests/e2e.rs`, `tests/api.rs`, and Web tests: lifecycle, persistence,
  repair, resume, SSE/report, and reducer contracts.

Tool authorization, workspace bounds, provider normalization, existing batch
ordering, and trace/SQLite ownership are unchanged.

## Non-Goals

- No immutable `PlanRevision` parent chain or `plan_revised` event.
- No `PlanEvaluator`, rule-first decision event, or model-based evaluator.
- No independent Finalizer or evidence-grounded final response phase.
- No public strategy/budget schema migration or global model/tool/token/cost
  enforcement.
- No dedicated step-record SQLite query table or fourth ledger artifact.
- No replay/reconstruction of an unknown in-flight external side effect.
- No trace-tail-to-task-state reconciliation after a torn cross-artifact write.

## Verification

Focused behavior:

```powershell
cargo test --test e2e planned_step_emits_complete_step_record_before_compatibility_completion -- --exact
cargo test --test e2e planner_resume_closes_unknown_in_flight_attempt_without_replay -- --exact
cargo test --test e2e planner_resume_applies_terminal_success_without_replaying_the_step -- --exact
cargo test --test e2e oneshot_persists_replanned_task_state -- --exact
cargo test --test api api_planned_tool_step_completes_after_successful_tool_call -- --exact
cargo test --test api api_restart_marks_pending_approval_interrupted_without_replaying_unknown_step -- --exact
```

Repository gates:

```powershell
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo check --features rag --bin rove-index
cd web-ui
pnpm test
pnpm typecheck
pnpm build
git diff --check
```

## Definition Of Done

- [x] Every emitted planned-step terminal outcome has one canonical record
  before its compatibility event.
- [x] Replanning retains earlier records and advances stable revision identity.
- [x] Task state, bounded checkpoint metadata, report, trace, SQLite repair,
  API/SSE, terminal, and Web consume the new contract.
- [x] Legacy serialized artifacts and legacy plan events remain readable.
- [x] Unknown in-flight work is closed as interrupted without automatic replay.
- [x] Repository-wide Rust, RAG, Web, and diff gates pass.
