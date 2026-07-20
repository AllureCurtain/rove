# Agent Execution Lifecycle Phase 1 Plan - 2026-07-19

> Status: **Implemented and verified - semantic foundation only**
>
> Baseline: `v0.1.0` / `b843448`
>
> Design source: [`../design/2026-07-14-agent-execution-lifecycle-design.md`](../design/2026-07-14-agent-execution-lifecycle-design.md)

> Follow-up: the bounded StepRunner was implemented in the next phase. The
> statement below that other budget dimensions were unset describes this
> Phase 1 baseline; planned execution now also resolves an independent
> `max_model_turns_per_step = 4` compatibility default. See
> [`2026-07-20-agent-execution-lifecycle-step-runner.md`](2026-07-20-agent-execution-lifecycle-step-runner.md).

## Objective

Establish the typed execution-lifecycle vocabulary that later StepRunner,
ledger, evaluator, persistence, and interface work can share. This phase must
preserve the `v0.1.0` runtime behavior while replacing implicit boolean
reasoning inside the core with an explicit, testable compatibility policy.

## Scope

This phase adds:

- `ExecutionStrategy` with the implemented values `react` and `plan_react`;
- a typed strategy-selection source;
- multidimensional budget limit and usage types;
- an explicit compatibility mapping from legacy `plan_enabled` and
  `max_steps`;
- semantic types for append-only `StepRecord`, versioned `PlanRevision`, and
  rule-first `PlanDecision` results;
- validation for budget, revision, and decision invariants;
- engine loop selection through the resolved strategy instead of reading the
  legacy boolean directly;
- focused serialization, compatibility, validation, and engine-routing tests.

## Compatibility Contract

The legacy limit keeps its historical unit and maps to exactly one new budget:

| Existing runtime mode | Resolved strategy | `runtime.max_steps` maps to |
|---|---|---|
| `plan_enabled = false` | `react` | `max_model_turns` |
| `plan_enabled = true` | `plan_react` | `max_step_attempts` |

Other budget dimensions remain unset in this phase. They are not silently
derived from the same legacy number, because that would give one field several
incompatible meanings.

This phase intentionally keeps the existing public fields and serialized
artifacts readable. It does not remove or deprecate `plan_enabled` or
`max_steps`; it creates the typed resolution boundary that a later config/API
migration will use.

## Non-Goals

- No bounded multi-turn StepRunner yet.
- No model-based evaluator, Replanner, or Finalizer calls.
- No new canonical lifecycle events.
- No `TaskState`, `PromptCheckpoint`, report, SQLite, or repair schema changes.
- No CLI/API/Web strategy selector or multidimensional budget controls.
- No capability snapshot or procedural-knowledge loading.
- No change to approval, workspace, tool-safety, cancellation, or resume
  semantics.

## Implementation Steps

1. Add `src/core/execution.rs` with the shared strategy, budget, ledger,
   revision, decision, finish-reason, and validation types.
2. Export the module from `src/core/mod.rs`.
3. Add `EngineConfig::execution_policy()` and route the existing planned and
   unplanned loops through its resolved `ExecutionStrategy`.
4. Add a read-only `RuntimeIdentity::execution_policy()` compatibility view;
   keep the persisted identity schema unchanged in this phase.
5. Add focused unit tests proving JSON names, legacy budget mapping, validation
   failures, and unchanged engine routing.
6. Update current runtime documentation only to describe the implemented
   compatibility boundary. Keep the larger lifecycle design marked
   `Proposed / Not Implemented`.

## Verification

Run the focused checks first:

```powershell
cargo test --lib core::execution
cargo test --lib core::runtime_identity
cargo test --test e2e planned_and_unplanned_runs_emit_equivalent_tool_lifecycle_events
```

Then run the default Rust gate:

```powershell
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
git diff --check
```

RAG and Web behavior should be unaffected because this phase adds no feature
or interface contract. Run broader gates if the implementation expands beyond
this scope.

## Definition Of Done

- [x] The semantic types and their validation rules compile and have focused
  tests.
- [x] Legacy `plan_enabled` and `max_steps` resolve deterministically without
  changing current loop behavior.
- [x] No persisted or external request schema changes are introduced.
- [x] Current runtime docs distinguish this compatibility foundation from the
  still-proposed StepRunner, ledger persistence, evaluator, and UI behavior.
- [x] The default Rust gate and `git diff --check` pass.
