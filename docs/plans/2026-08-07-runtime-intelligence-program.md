# Runtime Intelligence Program

> Status: **Ready for execution / Wave 1 active**
>
> Branch: `program/runtime-intelligence`
>
> Worktree: `.worktrees/runtime-intelligence`
>
> Base rule: start from the exact pushed `origin/main` commit supplied by the
> coordinator. Do not continue to another wave until the coordinator merges the
> current wave and supplies a refreshed base.

## 1. Mission

Complete the Runtime Intelligence lane of the post-Coding-Tool V2 program:

1. converge embedded and durable execution on one Runtime-neutral Agent kernel;
2. complete ambiguity evaluation, independent finalization, multidimensional
   budgets, reconciliation, lifecycle persistence, and product surfaces;
3. implement versioned AgentDefinition packages, immutable profiles, runtime
   `AGENTS.md` discovery, typed procedural knowledge, deterministic selection,
   and lifecycle integration;
4. build the deterministic OnCall reference Agent/evaluation suite.

The branch is long-lived across coordinator-controlled waves, but each wave is
a separate reviewed delivery. Execute only the active wave in this document.

## 2. Required Reading

Read before editing:

1. [`../../AGENTS.md`](../../AGENTS.md)
2. [`../ONBOARDING.md`](../ONBOARDING.md)
3. [`../runtime/README.md`](../runtime/README.md)
4. [`../runtime/react-loop.md`](../runtime/react-loop.md)
5. [`../runtime/implementation-status.md`](../runtime/implementation-status.md)
6. [`../design/2026-07-14-agent-execution-lifecycle-design.md`](../design/2026-07-14-agent-execution-lifecycle-design.md)
7. [`../design/2026-07-14-agent-definition-and-procedural-knowledge-design.md`](../design/2026-07-14-agent-definition-and-procedural-knowledge-design.md)
8. [`../design/2026-07-15-oncall-reference-agent-evaluation-plan.md`](../design/2026-07-15-oncall-reference-agent-evaluation-plan.md)
9. [`2026-08-07-post-coding-tool-v2-master-program.md`](2026-08-07-post-coding-tool-v2-master-program.md)

Then inspect the implementation and tests for every affected subsystem. Design
types are conceptual until implemented; current code and tests remain
authoritative.

## 3. Ownership and Boundaries

Primary Wave 1 ownership:

- `core/src/`
- `runtime/src/engine/`
- the minimum Runtime facade/executor/control/context integration needed for the
  kernel cutover;
- focused Core/Runtime tests plus `tests/embedding_contract.rs`, `tests/e2e.rs`,
  `tests/event_contract.rs`, and architecture guards;
- current runtime documentation changed by the implementation.

Do not edit Protocol & Platform primary ownership in Wave 1:

- MCP transport/proxy/dispatcher implementation;
- MCP product routes and MCP-specific Web settings;
- future Tool Artifact storage or rich MCP content mapping;
- Desktop/Tauri files.

Treat canonical serialized events, shared state schemas, ProductStore/API
contracts, root Cargo metadata/lockfiles, and broad Web types as coordinator
hotspots. If a hotspot change is truly required, stop that part of the work and
report the exact minimal contract, producer/consumer list, migration need, and
tests to the coordinator. Continue independent work where possible.

## 4. Active Work: Wave 1 One-Kernel Cutover

### 4.1 Characterize before changing

First prove and document the existing call paths:

- embedded `rove_core::Agent` execution;
- Runtime unplanned loop;
- Runtime planned coordinator and StepRunner;
- CLI, API, Web product jobs, TUI, and benchmark assembly;
- model/tool turns, control messages, approvals/input, hooks, event emission,
  persistence, cancellation, resume, and final reports.

Add focused characterization tests for any behavioral contract not already
protected. Do not infer a contract only from prose.

### 4.2 Establish the shared kernel boundary

Make one Runtime-neutral Agent kernel own multi-turn model/tool coordination.
The durable Runtime may wrap and observe the kernel, but must not retain a
second independent loop with divergent parsing, tool batching, cancellation,
or stop semantics.

The shared boundary must support:

- normalized model turns and native/compatibility tool calls;
- serial/parallel-safe tool batches through the authoritative registry;
- cancellation, steering, follow-up, approval, and input control;
- bounded step/turn outcomes and typed stop reasons;
- before/after extension hooks without provider- or product-specific payloads;
- canonical data needed by durable orchestration without importing SQLite,
  Axum, Clap, Ratatui, ProductStore, workspace state, or local tool authority
  into `rove-core`.

### 4.3 Preserve Runtime authority

Runtime continues to own:

- persistent task state, trace, report, checkpoint, repair, and resume;
- planning, StepRecord/PlanRevision/PlanDecision facts, and current rule-first
  evaluator behavior;
- workspace, memory/context/compaction, Execution Environment, tools/MCP,
  approval policy, and Project Trust;
- canonical Runtime stream events and product integration.

Do not solve later lifecycle, AgentDefinition, procedure, or OnCall phases in
Wave 1. Introduce an abstraction only where the one-kernel cutover requires it.

### 4.4 Compatibility requirements

- Existing public message, Tool, Agent, Engine, event, state, and report
  contracts remain compatible unless an additive defaulted migration is
  explicitly justified.
- Provider-specific data stays behind `rove-models`.
- Completed tool effects and plan steps are never replayed on resume.
- Pending approval/input and unknown in-flight effects remain fail closed.
- Local Fake Model and deterministic benchmarks remain network-free.
- CLI/API/Web/bench continue to assemble the shared Runtime rather than new
  private loops.

## 5. Wave 1 Verification

Run focused checks while iterating, including at least:

```powershell
cargo test -p rove-integration-tests --test embedding_contract
cargo test -p rove-integration-tests --test e2e
cargo test -p rove-integration-tests --test event_contract
cargo test -p rove-runtime
```

Before handoff run:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Run Web tests/typecheck/build if a consumed API/event/type contract changes,
and browser E2E if browser-visible lifecycle behavior changes. Report every
command and real exit result.

## 6. Wave 1 Exit Gate

Stop and hand back to the coordinator when:

- one kernel demonstrably owns embedded and durable multi-turn coordination;
- no duplicate Runtime loop remains for the migrated behavior;
- planned/unplanned, cancellation/control, approval/input, tool ordering,
  persistence, resume, and final-output regressions are covered;
- current runtime docs match the new ownership boundary;
- focused and required full gates pass;
- all work is committed and pushed to `program/runtime-intelligence`;
- `git status --short` is clean apart from explicitly reported user-owned
  files.

Do not start Wave 2. Provide commit SHAs, base SHA, changed files, compatibility
story, test exits, unrun optional gates, risks, and any shared-hotspot request.

## 7. Later Waves - Do Not Start Without Refresh

### Wave 2

Implement model-on-ambiguity evaluation, independent Finalizer, deterministic
fallback, public multidimensional budgets, global enforcement, trace-tail
reconciliation, persistence/resume/repair, events, configuration, and product
surfaces.

### Wave 3

Implement AgentDefinition/Profile, package validation and hashing, legacy
mapping, runtime identity, bounded root/nested `AGENTS.md` discovery, procedure
schema/trust/catalog/selection, and progressive hydration.

### Wave 4

Integrate procedures with planning/execution/finalization and implement the
versioned deterministic OnCall fixtures, oracles, evidence package, direct-tool
baseline, lifecycle/procedure/MCP scenarios, safety gates, resume, failures,
ablations, and optional provider experiments.

The coordinator will revise this brief with exact bases and shared contracts at
each barrier.
