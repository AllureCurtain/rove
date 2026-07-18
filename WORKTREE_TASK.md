# TUI Integration Coordinator Task

> Branch: `feat/tui-mvp`
>
> Foundation: `48375704aff6b7bfb8218d0dd9aaaf85f5d412c5`
>
> Role: sole integration and merge authority.

## Read First

Read completely:

1. `AGENTS.md`
2. `docs/ONBOARDING.md`
3. `docs/runtime/README.md`
4. `docs/design/2026-07-16-grok-build-reference-and-tui-design.md`
5. `docs/plans/2026-07-18-tui-parallel-worktree-handoff.md`

This worktree already contains the verified Foundation Commit. Do not recreate
worktrees or redo Foundation work.

## Active Workers

| Worktree | Branch | Task-brief commit |
|---|---|---|
| `../tui-state` | `feat/tui-state` | `c85384e` |
| `../tui-render` | `feat/tui-render` | `d00f77a` |
| `../tui-io` | `feat/tui-io` | `b723450` |

Each worker must produce one later implementation commit and report that SHA.
Cherry-pick only the implementation commit, not the task-brief commit.

## Coordinator Responsibilities

While workers are active:

- keep this worktree clean;
- do not edit worker-owned files;
- inspect worker status read-only when needed;
- reject scope expansion into core events, persistence, API, or Web unless it
  is explicitly evaluated as a cross-interface contract change.

After all worker implementation SHAs arrive:

1. inspect every commit with `git show --stat --oneline <sha>` and `git show`;
2. cherry-pick one implementation commit at a time;
3. run that worker's focused checks after each cherry-pick;
4. reconcile state/renderer interfaces in integration-owned files;
5. implement `rove tui`, CLI dispatch, the `tokio::select!` application loop,
   runtime effect execution, cancellation, and terminal-safe tracing behavior;
6. route engine events through the shared `terminal::run` driver and
   `RunViewState`; do not create TUI-only artifact or event persistence;
7. preserve existing REPL and `rove exec` behavior;
8. verify the fake-provider vertical slice and all Rust gates;
9. update `docs/runtime/` only after the behavior exists and passes tests;
10. remove this temporary `WORKTREE_TASK.md` before final integration.

## Integration-Owned Files

Only the coordinator may modify:

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

Small integration edits to worker-owned files are allowed only after their
commits are cherry-picked and reviewed.

## Known Decisions

- Approval can use the renderer-neutral interaction broker.
- The existing `input_needed` asymmetry must be resolved deliberately before
  the input modal; do not hide it behind a private TUI lifecycle.
- Tracing must not write uncontrolled output over the alternate screen.
- `Ctrl+C` cancels active work; exit and panic paths must restore the terminal.
- The first milestone is single-session and single-active-run only.

## Final Verification

At minimum run:

```powershell
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test interfaces::terminal --lib
cargo test interfaces::tui --lib
cargo test --test cli_repl
cargo test
git diff --check
git status --short
```

Report all integrated commit SHAs, conflicts and resolutions, exact test
results, behavior not yet implemented, and final worktree status.
