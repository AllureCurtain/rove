# Post-Coding-Tool V2 Checkpoint 1 - Shared Agent Kernel

> Status: **Implemented**
>
> Branch: `program/full-delivery`
>
> Parent checkpoint: `fe2ebfb49ff1ae1661b6023d3f3f926c389a7529`
>
> Recorded: 2026-08-07

## Implemented contract

`rove-core::run_agent_kernel` is the single Runtime-neutral model/action/tool
state machine used by:

- the public in-memory `rove_core::Agent` embedding host;
- the unplanned durable Runtime host in `runtime/src/engine/run_loop.rs`; and
- each bounded planned-step host in `runtime/src/engine/step_runner.rs`.

The kernel owns multi-turn progression, normalized `Action` transitions,
model/tool counters, cancellation boundaries, whole-batch tool-budget
reservation, tool-event forwarding, history progression, malformed-action
repair, and final/follow-up transitions. Its callback extension plane exposes
before/after model, tool, and final boundaries without importing Runtime types.

Runtime remains authoritative for prompt construction, context/compaction,
Workspace and Execution Environment, Project Trust, approval/input channels,
Runtime hooks and Executor, durable steer lifecycle facts, canonical
`StreamEvent`, planning, persistence, resume, reports, and product assembly.
There is no second interface-specific loop or state/event authority.

## Compatibility and safety

- Existing `Agent`, `AgentConfig`, `AgentControl`, `AgentEvent`, and
  `AgentOutcome` APIs remain available.
- Native and compatibility tool calls still cross the same normalized Core
  model-turn boundary. Provider-specific payloads remain in `rove-models`.
- Runtime tool actions still use the existing authoritative registry,
  approval/input path, hooks, Execution Environment, and ordered history
  projection.
- Approval/input events are forwarded while execution is waiting; they are not
  buffered until tool completion.
- A model-requested batch reserves its complete tool-call count before any
  dispatch. Embedded parallel execution is allowed only when every descriptor
  is non-destructive and `parallel_safe`; results are written back in request
  order. Other batches remain serial and stop after the first failed call.
- Cancellation remains active during model, policy, approval/input, tool, and
  post-run waits. Runtime completion still drops accepted-but-unapplied steers
  as canonical facts.
- The Core package still depends only on `rove-models` and creates no durable
  state.

## Verification evidence

Focused checks completed with exit code 0:

| Command or test slice | Result |
|---|---|
| `cargo test -p rove-core` | 19 passed |
| `cargo test -p rove-integration-tests --test embedding_contract` | passed |
| `cargo test -p rove-integration-tests --test workspace_architecture` | 4 passed |
| full `cargo test -p rove-integration-tests --test e2e` | 100 passed |
| focused approval/input/cancellation tests | passed |
| focused planned/unplanned parity and batch-order tests | passed |
| `cargo fmt --all --check` | passed |
| `cargo clippy --workspace --all-targets -- -D warnings` | passed |
| `cargo test --workspace` | passed |
| `pnpm test` | 34 files, 218 tests passed |
| `pnpm typecheck` | passed |
| `pnpm build` | passed |
| `scripts/product-acceptance.ps1` with a temporary report | PASS: 11 passed, 0 failed, 1 optional not run |
| `scripts/integration-smoke.ps1` | live local API/default shell: 5 passed |
| `rove-bench --suite agent-smoke` | 3/3 tasks passed |
| `rove-bench --suite coding-tool-v2` | 1/1 task passed, 13 tool calls, 0 failures |

The Core suite includes direct coverage for embedded safe-parallel and forced
serial batches, ordered completion events, pre-dispatch batch-budget failure,
before/after policy callbacks, steering, follow-up, and cancellation. The
architecture guard requires embedded, unplanned, and planned-step execution to
call the shared kernel and rejects a private model-action match in those hosts.

The product-acceptance browser entry passed its 56 deterministic scenarios with
5 opt-in scenarios skipped. The live local API suite then passed all five fake
provider scenarios. Generated acceptance and benchmark evidence is ignored or
stored outside Git. The optional official filesystem MCP smoke was not enabled
and is not interoperability evidence. External-provider and native Windows
PTY/platform gates were not run for this Runtime-only checkpoint.
