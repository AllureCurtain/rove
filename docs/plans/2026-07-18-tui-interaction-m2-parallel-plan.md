# TUI Interaction M2 Parallel Plan - 2026-07-18

> Status: **Active / Foundation In Progress**
>
> Scope: complete live approval and `request_input` interaction in `rove tui`.
> This plan does not claim that M2 is implemented until code, tests, and
> `docs/runtime/` agree.

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

## Frozen Modal Semantics

The parallel lanes must implement the same interaction contract:

- approval accepts `Y` only from a real `KeyEventKind::Press` and rejects on
  `N` or `Esc` from a real press;
- Enter, key repeat, and paste must never authorize a destructive action;
- input Enter submits the exact draft, including empty or whitespace-only
  text; input `Esc` is a no-op in M2;
- input characters, backspace repeats, and paste are accepted up to a 32 KiB
  UTF-8 byte limit without splitting a character;
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
