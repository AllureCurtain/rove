# TUI Parallel Worktree Handoff - 2026-07-18

> Status: **Completed Historical Handoff / First vertical slice implemented**
>
> Lifecycle: historical. The first vertical slice is integrated and described in
> `docs/runtime/implementation-guide.md`; later TUI milestones may use this file
> as execution history, not as current runtime truth.

This document is the handoff for a new main conversation that will create Git
worktrees, dispatch work to separate conversations, integrate their commits,
and verify the first full-screen TUI milestone.

It does not replace:

- [repository rules](../../AGENTS.md);
- [maintainer onboarding](../ONBOARDING.md);
- [current runtime documentation](../runtime/README.md);
- [the partially implemented TUI target design](../design/2026-07-16-grok-build-reference-and-tui-design.md).

## 1. Objective

Deliver this first vertical slice without changing the existing REPL or exec
contracts:

```text
rove tui --model fake
```

The command must start a full-screen terminal UI, accept one prompt, display
streamed runtime state, support cancellation and exit, preserve the existing
trace/task/report artifacts, and restore the terminal on every exit path.

Approval, `request_input`, session navigation, and PTY hardening follow after
this vertical slice. Do not implement AgentDefinition, MCP evolution,
background tasks, subagents, or new execution semantics as part of the TUI.

## 2. Current Repository State

Verified on 2026-07-18:

- current branch: `main`;
- current HEAD: `9e95b6b240d79cc07df5f8b5969278547170f611`;
- remote `origin/main` points to the same commit;
- only the main worktree exists;
- `.worktrees/` exists and is ignored;
- the legacy design/plan directory migration into `docs/design/` and
  `docs/plans/` is present locally but is not committed;
- `AGENTS.md`, `docs/ONBOARDING.md`, the active future designs, and this file
  are currently untracked from Git's perspective;
- `CODING_TASK_ROVE_36_42_PLAN.md` and the Yan Agent HTML analysis are
  user-owned untracked files and must not be included without explicit user
  confirmation.

Focused baseline checks passed:

```text
cargo test interfaces::terminal --lib  -> 3 passed
cargo test --test cli_repl              -> 7 passed
```

No TUI implementation code or worktree was created during planning.

## 3. Mandatory Baseline Step

New worktrees contain committed Git state only. Creating them before the
documentation migration is committed would omit the current `AGENTS.md`, TUI
design, and plan paths.

The new main conversation must first:

1. Read the four documents linked at the top of this file.
2. Run `git status --short`, `git diff --check`, and inspect every overlapping
   modified file.
3. Confirm with the user that the documentation migration may be committed.
4. Stage only the intended repository rules and documentation migration.
5. Keep unrelated user-owned files untracked and unstaged.
6. Commit the documentation baseline before creating implementation worktrees.

Candidate staging scope, subject to inspection:

```powershell
git add -u -- README.md docs
git add -- AGENTS.md docs/ONBOARDING.md docs/design docs/plans
git status --short
git diff --cached --check
git diff --cached --stat
```

Do not stage these unless the user separately authorizes them:

```text
CODING_TASK_ROVE_36_42_PLAN.md
Yan-Agent-*.html
ignored root-local tool state
.rove/
target/
```

A suitable baseline commit message is:

```text
docs: reorganize design and implementation plans
```

## 4. Worktree Topology

Use one integration worktree and three worker worktrees. Never run two
conversations in the same worktree.

```text
main
  `-- feat/tui-mvp              .worktrees/tui-mvp
        |-- feat/tui-state      .worktrees/tui-state
        |-- feat/tui-render     .worktrees/tui-render
        `-- feat/tui-io         .worktrees/tui-io
```

Create the integration worktree after the baseline commit:

```powershell
git worktree add .worktrees/tui-mvp -b feat/tui-mvp main
git worktree list
```

The main integration conversation owns `.worktrees/tui-mvp`. It first creates
and commits the foundation described below. All worker worktrees must then be
created from that exact foundation commit, not directly from `main`.

## 5. Foundation Commit

The integration conversation completes this step serially so every worker sees
the same contracts:

- add upstream `ratatui` and `crossterm` dependencies;
- create the `src/interfaces/tui/` module skeleton;
- define initial `TuiState`, `TuiAction`, and `TuiEffect` boundaries;
- predeclare the worker-owned modules so workers do not edit shared `mod.rs`
  files;
- make CLI runtime construction accept interface-provided approval/input
  providers while preserving stdin defaults for the existing REPL and exec;
- define the reusable run-event driver boundary that will keep artifact
  recording and finalization independent of a concrete renderer;
- add contract tests and keep the existing terminal and CLI tests green.

Central files owned only by the integration conversation:

```text
Cargo.toml
Cargo.lock
src/main.rs
src/interfaces/mod.rs
src/interfaces/cli/args.rs
src/interfaces/cli/runtime.rs
src/interfaces/tui/mod.rs
src/interfaces/tui/app.rs
docs/runtime/
```

Foundation verification:

```powershell
cargo fmt --all --check
cargo test interfaces::terminal --lib
cargo test --test cli_repl
```

Commit the foundation, record its SHA as `<F0_SHA>`, then create workers:

```powershell
git worktree add .worktrees/tui-state -b feat/tui-state <F0_SHA>
git worktree add .worktrees/tui-render -b feat/tui-render <F0_SHA>
git worktree add .worktrees/tui-io -b feat/tui-io <F0_SHA>
git worktree list
```

Do not share `CARGO_TARGET_DIR` between worktrees. Run focused tests in worker
worktrees and reserve the full test suite for the integration worktree.

## 6. Worker Ownership

| Worker | Responsibility | Allowed files |
|---|---|---|
| `tui-state` | Pure state, reducer, focus, scrolling, and key mapping | `src/interfaces/tui/{action,effect,state,reducer,keymap}.rs` and their unit tests |
| `tui-render` | Ratatui layout, widgets, and `TestBackend` coverage | `src/interfaces/tui/render.rs`, `src/interfaces/tui/widgets/`, render tests |
| `tui-io` | Terminal lifecycle guard, bounded interaction broker, reusable run driver | assigned `src/interfaces/tui/{terminal,providers,run}.rs`, assigned `src/interfaces/terminal/` files, and `src/interfaces/cli/render.rs` only when required for extraction |

Workers must not edit central files, `src/core/events.rs`, `src/state/`, API,
Web, runtime documentation, or another worker's files. A worker that discovers
a required cross-boundary change must stop and report it to the integration
conversation instead of expanding scope.

### Worker prompt: state

```text
Work in D:\Study\project\agent\rove\.worktrees\tui-state on branch
feat/tui-state, based on <F0_SHA>. Read AGENTS.md, docs/ONBOARDING.md,
docs/runtime/README.md, the TUI design, and the parallel handoff.

Implement only the pure TUI state/reducer/keymap task assigned in the handoff.
Do not perform terminal I/O, run the Engine, render Ratatui widgets, or edit
central files. Add focused unit tests, run cargo fmt --all --check and the
focused tests, commit the result, and report the commit SHA, tests, status, and
any integration assumptions. Do not merge, rebase, pull, or delete branches.
```

### Worker prompt: renderer

```text
Work in D:\Study\project\agent\rove\.worktrees\tui-render on branch
feat/tui-render, based on <F0_SHA>. Read AGENTS.md, docs/ONBOARDING.md,
docs/runtime/README.md, the TUI design, and the parallel handoff.

Implement only the pure Ratatui renderer and widgets assigned in the handoff.
Render from the shared TuiState without runtime calls. Add TestBackend coverage
for normal, narrow, and minimal supported terminal sizes; ensure the composer
and status line do not overlap. Do not edit central files or other worker
modules. Run formatting and focused tests, commit, and report the SHA and exact
verification. Do not merge, rebase, pull, or delete branches.
```

### Worker prompt: terminal and run I/O

```text
Work in D:\Study\project\agent\rove\.worktrees\tui-io on branch feat/tui-io,
based on <F0_SHA>. Read AGENTS.md, docs/ONBOARDING.md,
docs/runtime/README.md, the TUI design, and the parallel handoff.

Implement only the terminal lifecycle, bounded interaction broker, and shared
run-driver task assigned in the handoff. Terminal cleanup must be RAII-based;
closed approval channels reject, and closed input channels return a typed
error. Preserve current REPL/exec rendering and artifact behavior. Do not
privately change the canonical input event contract; report that issue to the
integrator. Run formatting plus terminal/CLI focused tests, commit, and report
the SHA and exact verification. Do not merge, rebase, pull, or delete branches.
```

## 7. Known Integration Decisions

The integration conversation owns these cross-cutting decisions:

1. `build_cli_runtime` currently installs stdin approval/input providers. Raw
   mode cannot use those providers directly, so injection must be resolved
   before TUI runtime wiring.
2. `render_run_events` currently combines event consumption, artifact
   recording/finalization, and line output. Extract one shared run driver;
   never create a second TUI-only persistence path.
3. Approval events are emitted by core, but API input providers currently
   create `input_id` and append `InputNeeded` externally. Resolve this canonical
   event asymmetry before implementing the input modal.
4. `RunViewState` represents one run. `TuiState` must own transcript history,
   reset behavior, pending-modal clearing, focus, and scroll position.
5. Tracing currently writes to stderr. Do not let logs corrupt the alternate
   screen; select a controlled TUI logging behavior in the integration path.

## 8. Integration Protocol

Each worker returns a clean commit SHA. The integration conversation then:

1. checks that the integration worktree is clean;
2. reviews each worker commit before applying it;
3. cherry-picks one worker commit at a time;
4. runs that worker's focused tests after each cherry-pick;
5. resolves exports and wiring only in integration-owned files;
6. implements `rove tui`, the `tokio::select!` app loop, fake-provider prompt
   execution, stream projection, cancellation, and artifact finalization;
7. runs focused TUI, terminal, and CLI regression tests;
8. runs the complete Rust gate;
9. updates `docs/runtime/` only after behavior is implemented and verified.

Workers do not cherry-pick one another, merge into `feat/tui-mvp`, or rewrite
history. The integration conversation is the only merge authority.

## 9. First Milestone Acceptance

The first milestone is complete only when:

- `rove tui --model fake` starts without provider credentials;
- a prompt reaches the shared `Engine`;
- canonical events update the TUI through `RunViewState`;
- trace, task state, report, and SQLite behavior remain shared;
- `Ctrl+C` cancels an active run without corrupting the terminal;
- normal exit, error, cancellation, and panic paths restore raw mode, cursor,
  and alternate screen;
- normal and narrow layouts do not panic or hide the composer;
- existing `rove` REPL and `rove exec` behavior remains unchanged;
- no TUI-only runtime lifecycle, event schema, or persistence format exists.

Required integration verification:

```powershell
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test interfaces::terminal --lib
cargo test --test cli_repl
cargo test
```

Do not run RAG or Web gates unless the implementation actually changes those
surfaces.

## 10. Follow-Up Order

After the first vertical slice:

1. add approval modal wiring;
2. resolve the canonical `input_needed` contract and add input modal wiring;
3. add sessions, resume, tool detail, scrolling, resize, and minimal-size
   fallback;
4. add PTY smoke tests, terminal restore tests, Windows/Unix checks, and help;
5. update current runtime documentation and retire this temporary handoff.

Do not parallelize changes that all modify `tui/app.rs` or the core event
contract. Parallel work is useful only while file ownership remains disjoint.

## 11. Prompt For The New Main Conversation

Use this as the first instruction in the new main conversation:

```text
Read AGENTS.md and docs/plans/2026-07-18-tui-parallel-worktree-handoff.md
completely. You are the sole TUI integration coordinator. First inspect the
dirty main worktree and prepare the documentation baseline without staging or
committing unrelated user files. Confirm the commit scope with me, then create
the integration worktree and execute the Foundation Commit section. Do not
create worker worktrees until the foundation commit passes its focused gates.
After that, create the three worker worktrees and give me the exact worker
prompts with the real F0 commit SHA substituted.
```
