# TUI Interaction M2 Parallel Plan - 2026-07-18

> Status: **Completed / Implemented (2026-07-19)**
>
> Scope: complete live approval and `request_input` interaction in `rove tui`.
> The implementation, focused and full Rust tests, and current runtime docs now
> agree on the capability-gated behavior described below.

## Objective

Deliver one coherent interaction lifecycle without adding a TUI-only runtime
event model:

```text
provider registers responder
  -> Core publishes one canonical waiting event
  -> live TUI opens a modal
  -> typed action resolves the matching responder once
  -> tool continues, fails closed, or is cancelled
```

## Integrated Checkpoint

All worker branches started from foundation commit `5cf976b` and were
integrated on `feat/tui-interaction-m2` in this order:

1. foundation verification docs: `7f00db2`;
2. interaction I/O: `2408921` (from worker `57d22cc`);
3. modal renderer: `0a33310` (from worker `c8daa43`);
4. modal state: `2d08e88` (from worker `d2852f9`).

The coordinator then added the runtime wiring, lifecycle cleanup, capability
gating, arming boundary, and regression tests in the same integration branch.
The final Rust gate passed `cargo fmt --all --check`,
`cargo clippy --all-targets -- -D warnings`, `cargo test`, and
`git diff --check`. The full test run included 261 library tests, 53 API tests,
88 E2E tests, and the remaining integration suites.

## Foundation Owned By The Coordinator

Before parallel work begins:

- use the `request_input` tool call ID as the stable `input_id`;
- make Core the only producer of canonical `InputNeeded` events;
- split input registration from awaiting the response so the provider is
  answerable before the event is visible;
- preserve API pending-row, SSE, trace, cancellation, and replay behavior;
- keep responder senders outside cloneable `TuiState`;
- freeze modal view/action types and file ownership.

Compatibility constraints for the foundation:

- keep the existing public `ToolContext` and `UserInputRequest` struct fields;
- retain the one-phase `UserInputProvider::request_input` and
  `ToolApprovalProvider::decide` methods for existing implementors and direct
  callers;
- require Engine-backed providers to implement the new two-phase
  `begin_input` and `begin_approval` methods before they can participate in
  canonical waiting-event ordering;
- keep the registered-input event sender crate-private so tools cannot forge
  lifecycle events.

## Parallel Lanes

All worker branches start from the same verified foundation commit.

| Lane | Owned files | Responsibility |
|---|---|---|
| Modal state | `src/interfaces/tui/{action,effect,state,reducer,keymap}.rs` | Modal state, typed actions, key routing, bounds, reducer tests |
| Modal renderer | `src/interfaces/tui/render.rs`, `src/interfaces/tui/widgets/` | Approval/input overlays and TestBackend coverage |
| Interaction I/O | `src/interfaces/terminal/interaction.rs`, `src/interfaces/tui/providers.rs` | Bounded transport, stable IDs, responder lifecycle, fail-closed tests |
| Coordinator | Core/API/CLI contract, `src/main.rs`, `src/interfaces/tui/app.rs`, integration tests and docs | Runtime wiring, cherry-picks, lifecycle cleanup, final verification |

Workers must not edit `app.rs`, `main.rs`, runtime documentation, or another
lane's files. Each worker commits its result and reports the commit SHA and
focused checks.

## Implemented Modal Semantics

The integrated implementation follows this interaction contract:

- On a direct-capability terminal (non-Windows with keyboard enhancement),
  approval accepts `Y` only from a fresh real `KeyEventKind::Press` and rejects
  on `N` or `Esc` from a real press. Input submits on a fresh `Enter` press.
- On Windows native events, approval `Y` only stages a selection and a fresh
  non-text `F8` press confirms it. Input submits with a fresh `F8` press;
  `Enter` does not submit it.
- On terminals without a trustworthy key-event mode, approval is rejected and
  input returns a typed unavailable error without opening a modal.
- For all modes, repeat, release, paste, wrong IDs/types, and actions received
  before the modal's visible-frame arming boundary cannot resolve an
  interaction.
- Input preserves the exact draft, including empty or whitespace-only text;
  input `Esc` is a no-op in M2.
- When a modal is actionable, input characters, backspace repeats, and paste
  are accepted up to a 32 KiB UTF-8 byte limit without splitting a character;
- while a modal is open, composer edits, focus changes, and transcript
  scrolling are blocked, while Ctrl+C and Ctrl+Q retain their global meaning;
- modal resolution and close operations match both the interaction variant
  and its ID;
- M2 adds no modal scrolling or selection state; the renderer wraps and clips
  static content and keeps the visible tail of an input draft;
- consumers skip queued requests whose responder is already closed, never
  overwrite a live responder, and drop the live responder on cancellation,
  completion, exit, EOF, draw failure, or terminal restoration failure.

## Safety And Acceptance

- approval and input requests are actionable only after responder registration;
- one live request produces exactly one canonical waiting event;
- IDs match across provider request, event, pending state, response, and tool
  completion;
- repeated keys, duplicate actions, wrong IDs, closed/full queues, dropped
  responders, cancellation, EOF, draw errors, and run completion fail closed;
- stale interactions never cross into the next run;
- default REPL, exec, API, SSE replay, artifacts, and Web event contracts remain
  compatible;
- focused tests, `cargo fmt --all --check`, Clippy with warnings denied, and the
  complete Rust test suite pass before current runtime docs are updated.

## Non-Goals

- session picker and session tabs;
- PTY and cross-platform terminal automation beyond regressions needed by M2;
- strict chronological transcript reconstruction;
- new permission policy, MCP, AgentDefinition, or background-task semantics.

## Completion Evidence

- Active-loop regressions cover the Windows `Y`-then-`F8` path, stale-key and
  paste/repeat rejection, and propagation of the selected interaction mode
  through `run_loop`.
- Terminal lifecycle tests cover Native, keyboard Enhancement, and Unavailable
  capability modes and restore every attempted terminal setting.
- The implementation remains single-session; session browsing, strict timeline
  reconstruction, mouse interaction, and PTY-level real-terminal automation are
  intentionally outside this milestone.
