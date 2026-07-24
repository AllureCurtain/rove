# Compact REPL UI Design

Date: 2026-05-31

## Purpose

Make `rove` feel like a real interactive agent when a user runs `rove` with no
message. The current REPL is functional but sparse: it prints a one-line banner,
uses the `rove>` prompt, and renders run events as plain bracketed lines. The
new design should keep the speed and low cognitive load of a normal terminal
while surfacing enough runtime context to make users confident about what rove
is connected to.

The chosen direction is a hybrid of two explored styles:

- the compact, developer-first feel of Claude Code-like terminal output;
- the product-specific context of rove's runtime console: workspace, model,
  provider, state, session, tool activity, and report path.

## Product Positioning

The REPL is the primary local command-line entry point for the rove runtime. It
should answer these questions immediately on startup:

- Where am I working?
- Which model/provider is active?
- Where will state be stored?
- Is this a new session or a resumed one?
- Which slash commands are available?

During runs it should answer:

- What is the current plan?
- Which tools ran?
- What results or failures occurred?
- How did the run finish?
- Where is the persisted report?

## Non-goals

This is not a full-screen TUI. Do not introduce panels that take over the
terminal, alternate screen buffers, mouse interactions, sidebars, or keyboard
navigation beyond the existing `rustyline` prompt.

This is not a replacement for the Web workbench. The Web UI remains the rich
history/report surface. The CLI REPL should be fast, line-oriented, and useful
inside a developer shell.

This design does not add dynamic runtime mutation commands such as `/model`,
`/provider`, or `/cwd`. Those are reasonable later additions, but this pass is
about presentation and status clarity.

## Visual Direction

Use this shape:

```text
rove
local-first agent runtime
workspace  Repo  D:\Study\project\agent\rove
model      Qwen/Qwen3-Coder-30B-A3B-Instruct
provider   openai-compatible
state      .rove  ·  session new

/help  /sessions  /resume latest  /status  /clear  /exit
rove> inspect this workspace and summarize the release blockers

You
  inspect this workspace and summarize the release blockers

Plan · 3 steps
  1. Read runtime docs and current status
  2. Check verification gates
  3. Summarize blockers and next steps

Tool · fs_read
  docs/runtime/release-readiness.md
  Read 184 lines

Tool · shell
  git status --short
  clean

Done · final
  3 steps · 2 tools · 0 failures · report .rove/runs/01KS.../report.json

rove>
```

Key traits:

- text-first, no large boxes by default;
- one compact metadata block on startup and `/status`;
- simple labels: `You`, `Plan`, `Tool`, `Result`, `Error`, `Done`;
- concise end summary with reason, steps, tools, failures, and report path;
- no verbose decorative framing inside normal runs.

## Terminal Styling

Keep styling optional and conservative.

Color is acceptable when stdout/stderr is a TTY and color is not disabled. The
implementation should also remain readable without color.

Recommended roles:

- title/accent: cyan;
- success/done/result: green;
- plan: yellow;
- path/detail: dim/muted;
- failure/error: red;
- normal assistant output: default terminal color.

Avoid icon/emoji dependencies. ASCII text should remain the canonical output so
Windows terminals, CI logs, and redirected output remain clean.

## Architecture

Add a focused CLI UI helper module and keep the REPL loop small:

- `src/interfaces/cli/ui.rs`: terminal style detection, compact banner/status
  formatting, slash command help text, and reusable line-format helpers.
- `src/interfaces/cli/render.rs`: event stream rendering remains responsible
  for artifacts and run event output. It should use small render-state counters
  for final summaries.
- `src/interfaces/cli/repl.rs`: REPL loop, slash command dispatch, history, and
  signal behavior. It should call UI helpers for startup and status output.

Do not move engine logic or state persistence into UI helpers. The UI layer is a
pure formatting boundary except for writing to stdout/stderr.

## Startup Banner

On `rove` with no message:

1. Build `CliRuntime`.
2. Enter `repl::run(runtime)`.
3. Print compact startup status.
4. Start reading lines with `rove>`.

The startup status should include:

- `workspace`: `WorkspaceKind` plus root path;
- `model`: `runtime.engine.model_id()` or config model;
- `provider`: `runtime.config.provider.name`;
- `state`: path relative to workspace root when possible, otherwise absolute;
- `session`: `new` initially.

For long model IDs or paths, truncate the middle or end only when needed. The
first implementation can use a simple character bound such as 96 chars for
single-line display. It should not panic on non-UTF-8 paths; use
`to_string_lossy()`.

## Status Command

Add `/status` as a first-class slash command.

`/status` should print the same compact status block as startup, with session
state reflecting the current `ReplState`:

- `session new` if no active resume state exists;
- `session resumed <short-run-id>` if `active_resume_state` is set;
- include the full run id in the line when space allows, or print
  `resumed 01KS...`.

Update `/help` to include `/status`.

## Run Rendering

The current renderer already receives structured `StreamEvent` values. It should
keep writing run artifacts and final reports exactly as today, while making
terminal output more intentional.

Event rendering rules:

- `LlmChunk`: stream the delta to stdout as-is.
- `RunStarted`: when in REPL mode, print a `You` block with the user message.
  One-shot output should not gain the REPL `You` block unless explicitly opted
  in.
- `PlanCreated`: print `Plan · N steps` and each step title, one per line.
- `PlanStepStarted`: avoid repeating step titles if the full plan was just
  printed. A compact implementation can skip this line by default or print it
  only when no plan has been printed.
- `ToolCallStarted`: print `Tool · <name>` plus arguments. For long JSON args,
  truncate to a safe length.
- `ToolCallCompleted`: print the result output, truncated to a safe length.
- `ToolCallFailed`: print `Error · <tool-name>` and the structured error.
- `RunCompleted`: print `Done · <reason>` and summary counters.

Summary counters:

- plan steps observed;
- tool calls started;
- tool failures;
- report path.

Use existing finalization path to know the run directory. The summary can print
`run_dir/report.json`, made relative to `workspace.root` when possible.

## One-shot Compatibility

Do not make one-shot mode noisy. Existing one-shot behavior is a product
contract: `rove --model fake "hello"` should still print the assistant/final
output in a simple way.

The shared renderer should support modes:

- one-shot mode: preserve current output shape as much as possible;
- REPL compact mode: print `You`, compact plan/tool/done blocks, and summary.

This can be represented by a new option enum such as:

```rust
pub enum CliRunRenderMode {
    OneShot,
    ReplCompact,
}
```

or by extending `CliRunRenderOptions`.

## Slash Commands

Keep current commands:

- `/help`
- `/exit`, `/quit`
- `/clear`
- `/sessions`
- `/resume latest`
- `/resume <run_id>`

Add:

- `/status`

When `/resume` succeeds, print a short status line and update the active resume
state. The next `/status` should reflect that resume state.

## Testing Strategy

Add focused tests for formatting helpers instead of snapshotting full terminal
sessions wherever possible. Suggested tests:

- startup/status formatter includes workspace kind, model, provider, state, and
  commands;
- slash parser recognizes `/status`;
- `/help` output mentions `/status`;
- `render_run_events` in REPL compact mode prints `Plan`, `Tool`, `Done`, and
  report path;
- one-shot mode still prints fake output without the startup banner;
- `tests/cli_repl.rs` no-arg smoke test expects the new banner text and can
  exit with `/exit`.

Keep tests deterministic and fake-provider only.

## Acceptance Criteria

The implementation is complete when:

- running `rove --model fake --approval never` enters a REPL with the compact
  status block;
- `/status` prints the same compact runtime status;
- `/help` lists `/status`;
- a fake run inside the REPL shows compact `You`, `Plan`, `Tool`, and `Done`
  sections when those events occur;
- one-shot fake runs still behave as simple one-shot commands;
- `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test`, and `cargo test --test cli_repl` pass.

## Risks

Terminal styling can easily become harder to read than plain text. Keep the
first pass restrained and line-oriented.

The shared renderer serves both REPL and one-shot paths. Use explicit render
mode/options to avoid accidental behavior changes in one-shot mode.

Windows path display can become noisy with verbatim prefixes. Prefer lossy
display plus relative paths where practical, and test on Windows-style paths
through formatting helpers.
