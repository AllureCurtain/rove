# Rich Terminal REPL Design

Date: 2026-06-09

## Purpose

Make the terminal experience feel closer to Claude Code and Codex while keeping
Rove's local-first runtime identity. The new primary CLI surface is a rich,
scrollback-preserving terminal REPL, not a fullscreen dashboard.

This design supersedes the one-shot compatibility rule in
`docs/design/2026-05-31-compact-repl-ui-design.md`: a bare positional
message no longer means "run once and exit". Bare messages start an interactive
session with an initial prompt.

## Product Decisions

Rove has one primary interactive CLI path:

```text
rove
rove "inspect this repo"
rove inspect this repo
```

All three enter the rich terminal REPL. When a message is present, Rove submits
it as the first turn, renders the run, and then stays in the interactive session
for follow-up input.

Non-interactive usage moves to an explicit subcommand:

```text
rove exec "inspect this repo"
```

`rove exec` inherits the current one-shot behavior: run the task, stream output,
persist trace/state/report artifacts, and exit. Script-focused options such as
`--json`, `--output-last-message`, and CI-friendly output belong under `exec`,
but JSON output is not part of the first implementation pass.

## Reference Behavior

Claude Code documents `claude` as an interactive session, `claude "query"` as an
interactive session with an initial prompt, and `claude -p "query"` as a
non-interactive query that exits:

https://docs.claude.com/en/docs/claude-code/cli-reference

Claude Code also treats fullscreen rendering as an opt-in alternate rendering
path. Its classic renderer keeps conversation text in the terminal's native
scrollback:

https://code.claude.com/docs/en/fullscreen

Codex documents `codex` with no subcommand as launching an interactive terminal
UI, accepts an optional prompt for the session, and uses `codex exec` for
non-interactive runs:

https://developers.openai.com/codex/cli/reference

Rove follows the same product split: interactive by default, explicit exec for
automation.

## Non-goals

Do not introduce alternate-screen rendering in this pass. The first version
must preserve normal terminal scrollback and must not use a ratatui/crossterm
fullscreen app shell.

Do not add mouse interactions, sidebars, fixed bottom input panes, transcript
navigation, theme pickers, or complex focus modes.

Do not remove the existing Web workbench. The Web UI remains the richer browser
surface for history, API-backed work, and inspection.

Do not broaden provider, model, approval, or workspace configuration beyond what
is required for the terminal entry-point and renderer changes.

## Interaction Model

Startup prints a compact status block, then either waits for input or submits
the initial prompt:

```text
Rove
workspace  D:\Study\project\agent\rove
model      qwen...
mode       interactive
session    new

> inspect this repo

User
  inspect this repo

Status
  reading workspace context

Plan
  1. Inspect CLI entry points
  2. Check renderer state
  3. Summarize required edits

Tool
  shell_command
  rg "run_oneshot|repl::run" src tests

Command
  cargo test --test cli_repl

Approval
  shell_command wants broader access
  approve / deny:

Input
  Which provider should be used?
  rove input>

Assistant
  ...

Done
  final - 3 steps - 2 tools - report .rove/runs/.../report.json

>
```

The renderer is text-first. It uses stable labels, indentation, optional ANSI
color, and concise summaries instead of heavy boxes or decorative panels. The
preferred visual direction is quiet, light, readable, and focused.

## Block Vocabulary

Every important runtime state should map to a terminal block:

- `User`: user-entered prompts and initial prompt submissions.
- `Status`: short progress updates from runtime state.
- `Plan`: plan creation, step start, step completion, and step failure.
- `Tool`: tool calls, key arguments, result summaries, and failures.
- `Command`: shell commands and test commands, with exit code and concise output.
- `Approval`: pending approval requests and the accepted or rejected answer.
- `Input`: pending `request_input` prompts and user answers.
- `Assistant`: final and meaningful intermediate natural-language output.
- `Done`: completion reason, step count, tool count, failure count, report path.

Shell tools should render through the same event pipeline as other tools, but
the terminal renderer should give shell and test commands a command-shaped
presentation because users scan them differently from generic tool calls.

## Architecture

Use the terminal view/action boundary that already exists on `main`.

`src/interfaces/terminal/view.rs`

- Remains the renderer-independent accumulator from `StreamEvent` to terminal
  view updates and view state.
- Adds or refines fields only when the rich renderer needs stable semantic
  information that should not be recovered from formatted strings.
- Keeps stream-to-view mapping testable without a real terminal.

`src/interfaces/terminal/action.rs`

- Remains the renderer-independent action model for prompt submission, slash
  commands, cancellation, approval, input answers, resume, status, sessions,
  clear, and exit.
- Acts as the shared vocabulary between the REPL loop and any later terminal
  renderer.

`src/interfaces/cli/args.rs` and `src/main.rs`

- Add an `exec` subcommand.
- Route bare positional messages into the interactive REPL as initial prompts.
- Route `exec` messages into the non-interactive backend.

`src/interfaces/cli/repl.rs`

- Owns readline, history, slash command dispatch, session state, signal
  handling, and initial prompt submission.
- Should stay small by delegating render details to the renderer and semantic
  state to `interfaces::terminal`.

`src/interfaces/cli/render.rs`

- Becomes the rich scrollback renderer for interactive runs.
- Consumes `RunViewUpdate` and `RunViewState` rather than duplicating event
  interpretation where practical.
- Keeps output useful without color and avoids terminal-specific assumptions.

`src/interfaces/cli/oneshot.rs`

- Becomes the implementation backend for `rove exec`, or is wrapped by a new
  `exec.rs` module with a clearer product name.
- No longer runs from bare positional messages.

## Data Flow

Interactive startup:

1. Parse CLI args.
2. Build `CliRuntime`.
3. Enter `repl::run(runtime, initial_prompt)`.
4. Print the rich welcome/status block.
5. If `initial_prompt` exists, add it to history as appropriate and submit it as
   the first run.
6. Render the run through `render_run_events` using rich REPL mode.
7. Load the latest `TaskState` after successful completion and keep the REPL
   active for follow-up input.

Exec startup:

1. Parse CLI args with `Command::Exec`.
2. Build `CliRuntime`.
3. Resolve optional resume state.
4. Start a run and call the existing one-shot backend.
5. Preserve the current non-interactive output contract unless explicitly
   changed by exec-specific flags.
6. Exit.

## Error Handling

Parser errors should make the new command split clear. Unknown subcommands
should remain clap errors. A missing `rove exec` prompt should fail with a
direct message such as "exec requires a prompt" unless stdin support is added in
the same implementation plan.

Interactive run failures should render as `Error` or failed `Tool`/`Command`
blocks and then return to the REPL when the runtime can recover. Fatal runtime
setup failures should exit before entering the REPL.

`Ctrl+C` while a run is active cancels the active run. Idle `Ctrl+C` behavior may
continue to match the current REPL behavior in the first implementation.

Redirected output must remain readable. Color and styling are optional
enhancements gated on terminal capability.

## Testing

Parser tests:

- `rove "hello"` parses as a positional initial prompt for interactive mode.
- `rove hello world` joins into one initial prompt for interactive mode.
- `rove exec "hello"` parses as non-interactive exec.
- Existing subcommands such as `sessions`, `state`, `index`, and `dump-config`
  keep their current parsing behavior.

REPL integration tests:

- `rove --model fake --approval never "hello"` prints the welcome/status block,
  renders the first run, accepts `/exit`, and exits successfully.
- The first run output includes `User`, `Assistant`, and `Done` blocks.
- The process does not exit immediately after the first run.

Exec integration tests:

- `rove exec --model fake --approval never "hello"` prints the fake final output,
  does not print the welcome/status block, does not wait for `/exit`, and exits
  successfully.
- Multi-word exec prompts preserve the current joined-message behavior.

Renderer tests:

- Tool, command, approval, input, plan, assistant, and done updates produce the
  expected block labels.
- Long command args and tool outputs are truncated without breaking Unicode.
- Output remains meaningful when ANSI styling is disabled.

Regression:

- Run `cargo fmt -- --check`.
- Run `cargo test`.

## Implementation Order

1. Add `exec` CLI parsing and adjust tests for the new command split.
2. Change `main.rs` routing so bare messages enter `repl::run` as initial
   prompts.
3. Adapt `repl.rs` to accept and auto-submit an optional initial prompt.
4. Wrap or rename the existing one-shot path as the `exec` backend.
5. Expand terminal view state only where rich blocks need stable semantics.
6. Upgrade REPL rendering block by block, keeping non-interactive output stable.
7. Update runtime docs and CLI help text.
8. Run full verification.

## Acceptance Criteria

- `rove` starts the rich interactive REPL.
- `rove "prompt"` starts the rich interactive REPL, runs the prompt, then waits
  for follow-up input.
- `rove exec "prompt"` runs non-interactively and exits.
- All planned blocks are represented in the interactive renderer or have an
  explicit event-level reason why the current runtime cannot emit them yet.
- No alternate-screen or fullscreen terminal framework is introduced.
- Existing run artifacts, trace writing, reports, resume behavior, and session
  state remain compatible.
- `cargo fmt -- --check` and `cargo test` pass.
