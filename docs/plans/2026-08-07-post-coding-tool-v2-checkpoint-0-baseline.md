# Post-Coding-Tool V2 Checkpoint 0 Baseline

> Status: **Checkpoint 0 implemented / Historical starting-state evidence**
>
> Branch: `program/full-delivery`
>
> Protected starting commit: `5bd38f3faded8a24b575649b65b6d6742e004f3a`
>
> Recorded: 2026-08-07

This record characterizes the exact starting state for the full-delivery
program. Later checkpoint commits intentionally change several entries below;
current behavior after those commits remains documented under `docs/runtime/`.

## Repository and package boundary

- `HEAD`, `origin/main`, `origin/program/full-delivery`, and their merge base
  all resolved to the protected starting commit above before any edit.
- The worktree was clean and the checked-out branch was
  `program/full-delivery`.
- Cargo metadata reported the local dependency direction
  `rove-models <- rove-core <- rove-runtime <- rove-app-bootstrap <-`
  `{rove-cli, rove-api, rove-bench}`, plus `rove-integration-tests`.
- `rove-models` had no local dependency. `rove-core` depended only on
  `rove-models`; persistence, HTTP, CLI, and TUI crates were absent from its
  dependency tree.

## Starting execution map

| Concern | Starting authority and evidence |
|---|---|
| Embedded Agent | `core/src/agent.rs` owned an in-memory multi-turn loop, control queues, tool policy, and `AgentEvent`/`AgentOutcome`. |
| Durable execution | `runtime/src/engine/facade.rs` selected the planned or unplanned Runtime loop; `run_loop.rs`, `plan_loop.rs`, and `step_runner.rs` retained durable coordination separate from `core::Agent`. |
| Model turns | `runtime/src/engine/model_turn.rs` translated the shared `rove_core::model_turn::run_model_turn` output into canonical Runtime events. |
| Tool turns and safety | `runtime/src/engine/tool_turn.rs` and `runtime/src/tools/executor.rs` owned Runtime services, approval/input, hooks, observed mutations, and the Execution Environment over the authoritative Core registry. |
| Controls | Core exposed in-memory cancel/steer/follow-up queues. Runtime/API/ProductStore owned durable safe-point steer and follow-up facts plus approval/input channels. |
| Lifecycle | Bounded StepRunner, append-only `StepRecord`, immutable `PlanRevision`, and rule-first `PlanDecision` existed. Model-on-ambiguity, an independent Finalizer, full multidimensional budgets, and trace-tail reconciliation did not. |
| Events | `runtime/src/foundation/events.rs` was the canonical lifecycle consumed by trace, SQLite, CLI/TUI, API SSE, Web, reports, and contract tests. |
| Durable truth | `trace.jsonl` held event facts, `task_state.json` held resumable projections, `report.json` was derived, and SQLite remained a rebuildable query index. |
| MCP | `runtime/src/tools/mcp_proxy.rs` supported stdio and legacy SSE with duplicated transport request paths and a text-oriented result projection. Streamable HTTP, negotiated sessions, shared dispatch, rich results, and durable Tool Artifacts were absent. |
| Tool output | `core/src/tools.rs` exposed `ToolOutput { content, mutations }`; transient Coding Tool artifact projections were not durable Tool Artifacts. |
| Product assembly | CLI and API called `apps/bootstrap::build_engine`; benchmark code used the same Runtime `Engine`; first-party apps did not construct a private Core Agent loop. |
| Web | `apps/web` consumed API/SSE and ProductStore contracts. Its production build initially depended on build-time Google Font downloads. |
| Benchmark | Benchmark V1 used scripted Fake turns, the real Runtime/ToolRegistry/StateStore path, deterministic checks, and cancel/resume; AgentDefinition/procedure/OnCall V2 did not exist. |
| Desktop | `apps/desktop` and a Tauri host did not exist. |

`tests/workspace_architecture.rs` now guards the first-party Engine assembly and
Core model-normalization boundary. `tests/code_hygiene.rs` also prevents the
deterministic Web production build from regaining a remote-font dependency.

## Starting gates

The first direct `pnpm test` attempt could not start because this new worktree
had no `node_modules`. `pnpm install --frozen-lockfile` completed without a
lockfile change. The first `pnpm build` then failed because `next/font/google`
could not fetch Geist. Checkpoint 0 removed that network dependency and used
the existing cross-platform system font stacks; the repeated build passed.

The following commands then completed with exit code 0:

| Command | Result |
|---|---|
| `cargo fmt --all --check` | passed |
| `cargo clippy --workspace --all-targets -- -D warnings` | passed |
| `cargo test --workspace` | passed |
| `pnpm test` from `apps/web` | 34 files, 218 tests passed |
| `pnpm typecheck` from `apps/web` | passed |
| `pnpm build` from `apps/web` | passed without network font fetch |
| `scripts/product-acceptance.ps1` with a temporary report path | PASS: 11 passed, 0 failed, 1 optional not run |
| Product acceptance Playwright entry | 56 passed, 5 opt-in scenarios skipped |
| `scripts/integration-smoke.ps1` | passed; live local API/default shell 5/5 |
| `rove-bench --suite agent-smoke` | passed 3/3 tasks |
| `rove-bench --suite coding-tool-v2` | passed 1/1 task, 13 tool calls, 0 tool failures |

The product acceptance report was generated by the runner in the OS temporary
directory, not hand-edited or committed. Its provenance recorded the protected
starting commit and the five in-progress Checkpoint 0 files as dirty.

## Optional gate availability

- No external-provider smoke gate was enabled and no provider credential was
  present. External-provider interoperability was not tested.
- `ROVE_MCP_FILESYSTEM_SMOKE` was not enabled. The official filesystem MCP
  smoke was recorded as optional `not_run`; deterministic MCP fixtures passed.
- Real-API `local-full` passed all five deterministic fake-provider browser
  scenarios against an isolated API/workspace. This is not external-provider
  evidence.
- Windows has no native ConPTY automation in this baseline; the documented
  TUI PTY gate returned JSON `status: skipped` and captured exit code 77 on
  Windows. It is not platform evidence.
- No Desktop packaging/signing environment existed because no Desktop host
  existed at the starting commit.
