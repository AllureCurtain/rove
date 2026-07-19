# TUI Completion Parallel Plan - 2026-07-19

> Status: **Implementation lanes integrated / final verification and main merge pending**
>
> Baseline: `7d82df9` (`feat(tui): complete capability-gated interaction`)
>
> Integration branch: `feat/tui-completion`; lane commits are integrated and the
> coordinator records the final verification result before the `main` merge.
>
> This plan completes the TUI state-navigation and hardening work that was
> intentionally outside M2. The shared runtime, canonical `StreamEvent`
> lifecycle, artifacts, approval/input safety contract, and existing REPL/API
> behavior remain unchanged.

## Objective

Bring the full-screen TUI to the next complete MVP boundary:

- browse resumable task states and select a resume target inside the TUI;
- inspect tool calls and their output/error details without exposing hidden model
  reasoning;
- preserve a strict, renderer-neutral event timeline for the transcript;
- harden terminal setup/restore and add opt-in real-terminal/PTY verification;
- keep all behavior bounded, deterministic, and fail-closed; treat display
  sanitization as defense in depth rather than a proof of secret absence.

Mouse interaction and visual polish are welcome only when they fit the existing
typed action/reducer boundary. They must not introduce a second runtime loop or
change permission semantics.

## Parallel Lanes

### Lane A: navigation and overlays

Worktree: `.worktrees/tui-navigation`

Branch: `feat/tui-navigation`

Status: integrated as `2686fb9` from worker commit `0e0f67a`.

Owns the cloneable TUI interaction surface:

- session picker/resume selection backed by the bounded
  `StateStore::list_resumable_task_states_limited` query and
  `resolve_resume_state`-compatible identities;
- tool-detail overlay for a selected completed/failed tool call;
- a complete in-app key/help view that reflects the actual keymap;
- typed actions/effects, reducer state, focus rules, bounded list/detail
  rendering, and TestBackend/reducer tests.

Do not modify `src/interfaces/tui/terminal.rs`, `src/interfaces/terminal/view.rs`,
core event types, API contracts, or runtime persistence formats. Keep the
existing approval/input modal contract intact. If `app.rs` wiring is required,
keep it limited to dispatching the new typed effects; document integration
points for the coordinator.

Acceptance:

- picker opens only while idle, lists bounded resumable states newest-first, and
  handles empty, stale, malformed, and cancelled selections without panics;
- selecting a run sets the next prompt's resume state and never replays a
  completed side effect;
- tool detail contains only bounded, sanitized display projections; it does not
  intentionally copy raw hidden reasoning or unbounded provider payloads;
- help text is generated from the actual supported actions;
- focused tests cover wrong IDs, busy/active runs, narrow terminals, and exit.

### Lane B: renderer-neutral timeline model

Worktree: `.worktrees/tui-timeline`

Branch: `feat/tui-timeline`

Status: integrated as `37dcb7b` from worker commit `93d1667`.

Owns the renderer-independent terminal projection only:

- extend `RunViewState`/`RunViewUpdate` (or add a focused sibling module) with a
  bounded ordered timeline of visible lifecycle entries;
- assign stable sequence/order metadata from canonical updates without inventing
  a private event lifecycle;
- include user, assistant, safe model status, plan, tool, approval/input,
  compaction, memory, and completion entries with explicit typed variants;
- preserve existing aggregate fields used by CLI/API/Web and keep replay/resume
  behavior compatible;
- add focused unit tests for ordering, duplicate/idempotent updates, bounds,
  multi-run history, cancellation, and negative redaction canaries.

Do not modify `src/interfaces/tui/app.rs`, TUI reducer/keymap, terminal setup, or
API persistence. The coordinator will consume the timeline from the TUI
renderer after this lane lands.

Acceptance:

- timeline order follows canonical update delivery order, including events that
  arrive in the same model/tool turn;
- no raw thinking/reasoning, tool payload, or memory note is intentionally
  copied into timeline entries; representative secret-shaped canaries are
  covered by negative tests;
- old aggregate projections and all existing terminal/API tests remain green;
- bounds are explicit and deterministic under long runs.

### Lane C: terminal hardening and real-terminal verification

Worktree: `.worktrees/tui-hardening`

Branch: `feat/tui-hardening`

Status: integrated as `5303a16` from worker commit `0e4f679`.

Owns terminal lifecycle and opt-in verification:

- audit/strengthen raw mode, alternate screen, cursor, bracketed paste,
  keyboard enhancement, mouse capture (if enabled), and restore ordering;
- add deterministic capability-matrix tests for native/enhancement/unavailable
  modes and partial setup/restore failures;
- add an opt-in PTY smoke harness or platform-specific runner that launches
  `rove tui --model fake`, verifies nonblank frames, resize behavior, clean exit,
  and terminal restoration, without requiring provider credentials;
- document platform prerequisites and explicit skip behavior.

Do not change TUI state/reducer semantics or add a dependency unless the PTY
harness genuinely requires it and the dependency is justified in the plan.
Real-service/provider tests remain opt-in and secret-free.

Acceptance:

- every attempted terminal mode is restored on success, error, EOF, and panic;
- PTY checks are bounded, opt-in, and never point at production services;
- unsupported platforms report a typed skip/reason rather than a false pass;
- existing focused TUI and terminal tests remain green.

### Optional Lane D: release verification

Worktree: `.worktrees/tui-release-verification`

Branch: `feat/tui-release-verification`

Status: verification lane prepared; release evidence remains separate from the
implementation commits and may record explicit platform skips.

This lane may run independently while the code lanes work. It owns only
verification scripts/docs and evidence hygiene: Web checks, RAG feature checks,
integration-test classification, and a release checklist for the completed TUI.
It must not edit runtime code or claim real-service coverage when the gate was
skipped.

## Integrated Outcome

The three implementation lanes are now represented on `feat/tui-completion`:

- navigation/overlays: bounded session picker, resume revalidation, tool detail,
  and keymap-derived help;
- timeline: bounded renderer-neutral visible entries with canonical delivery
  order, deduplication, and redaction;
- hardening: terminal setup/restore recovery, capability matrix, and the
  opt-in Unix PTY smoke harness.

The coordinator still owns the final contract audit and full verification. In
particular, the live transcript must consume the timeline projection, resume
selection must exclude in-flight jobs, streamed hidden-reasoning filtering must
hold across chunk boundaries, and the PTY resize assertion must prove a redraw.
Those checks are required before declaring the integration branch merge-ready.

## Integration Rules

- The coordinator integrates worker commits only after focused tests and a
  diff/secret audit. No worker commits directly to `main`.
- `StreamEvent` remains the canonical lifecycle contract. No TUI-only event
  stream or persistence format may be introduced.
- Workspace paths remain bounded by the resolved workspace; provider text never
  grants permissions or becomes trusted instructions.
- Completed mutations and completed plan steps must not replay on resume.
- Runtime docs under `docs/runtime/` are updated only after behavior and tests
  are integrated.
- The integration branch is `feat/tui-completion`; final merge to `main` waits
  for all required lanes and the full Rust gate.

## Definition Of Done

- [x] Lane A and B behavior is integrated. The live transcript consumes the
  bounded timeline; navigation overlays use typed actions/effects and focused
  negative tests.
- [x] Lane C terminal lifecycle hardening and an opt-in Unix PTY harness are
  integrated. Windows records an explicit exit-code-77 skip because no native
  ConPTY runner exists; this is not a pass result.
- [x] Final `cargo fmt --all --check`,
  `cargo clippy --all-targets -- -D warnings`, `cargo test`, and relevant
  Web/RAG checks must pass on the integration head before merge to `main`.
- [x] Current runtime/design docs distinguish implemented behavior from optional
  or deferred platform/product work.

The coordinator completed the post-integration Rust, RAG, and Web gates before
merge. The Windows PTY harness recorded its documented exit-code-77 skip, not a
pass. This plan must not be used by itself as release or cross-platform
interoperability evidence.
