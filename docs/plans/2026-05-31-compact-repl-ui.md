# Compact REPL UI Implementation Plan

> **For implementers:** Execute this plan task by task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the compact rove REPL UI described in `docs/design/2026-05-31-compact-repl-ui-design.md`.

**Architecture:** Keep the REPL line-oriented and preserve one-shot CLI behavior. Add a focused `src/interfaces/cli/ui.rs` formatting layer, add `/status` in `repl.rs`, and extend `render.rs` with an explicit render mode so REPL output can be richer without making one-shot output noisy.

**Tech Stack:** Rust 2024, `rustyline`, existing `StreamEvent` renderer, existing CLI/runtime/state modules, no new UI crate required for the first pass.

---

## File Map

- Create `src/interfaces/cli/ui.rs`: pure formatting helpers for compact status, help text, short ids, path display, truncation, and optional style placeholders.
- Modify `src/interfaces/cli/mod.rs`: export the new `ui` module.
- Modify `src/interfaces/cli/repl.rs`: add `/status`, print compact startup banner, call UI helper for help/status, and pass REPL render mode into the shared renderer.
- Modify `src/interfaces/cli/render.rs`: add `CliRunRenderMode`, extend render options, track plan/tool/failure counters, and print compact REPL run sections while preserving one-shot behavior.
- Modify `src/interfaces/cli/oneshot.rs`: pass one-shot render mode explicitly.
- Modify `tests/cli_repl.rs`: update smoke expectations for compact banner and add `/status` smoke coverage.
- Modify `docs/runtime/implementation-guide.md`: update REPL description and command list.

## Task 1: Add UI Formatting Helper

**Files:**
- Create: `src/interfaces/cli/ui.rs`
- Modify: `src/interfaces/cli/mod.rs`

- [ ] **Step 1: Write formatting tests**

Create `src/interfaces/cli/ui.rs` with tests first. Use a minimal public API that later tasks can call:

```rust
use std::path::{Path, PathBuf};

use crate::config::AppConfig;
use crate::core::types::TaskState;
use crate::core::workspace::{Workspace, WorkspaceKind};

pub struct ReplStatusView<'a> {
    pub workspace: &'a Workspace,
    pub config: &'a AppConfig,
    pub model_id: &'a str,
    pub active_resume_state: Option<&'a TaskState>,
}

pub fn format_repl_status(_view: ReplStatusView<'_>) -> String {
    String::new()
}

pub fn format_repl_help() -> String {
    String::new()
}

pub fn short_id(value: impl AsRef<str>) -> String {
    value.as_ref().to_string()
}

fn display_path(path: &Path, workspace_root: &Path) -> String {
    path.strip_prefix(workspace_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use crate::config::AppConfig;
    use crate::core::workspace::Workspace;

    use super::{format_repl_help, format_repl_status, short_id, ReplStatusView};

    #[test]
    fn repl_status_includes_runtime_context_and_commands() {
        let tmp = TempDir::new().unwrap();
        let workspace = Workspace::detect(tmp.path()).unwrap();
        let mut config = AppConfig::default();
        config.provider.name = "openai-compatible".to_string();
        config.provider.model = "test-model".to_string();

        let output = format_repl_status(ReplStatusView {
            workspace: &workspace,
            config: &config,
            model_id: "test-model",
            active_resume_state: None,
        });

        assert!(output.contains("rove"));
        assert!(output.contains("local-first agent runtime"));
        assert!(output.contains("workspace"));
        assert!(output.contains("folder"));
        assert!(output.contains("model"));
        assert!(output.contains("test-model"));
        assert!(output.contains("provider"));
        assert!(output.contains("openai-compatible"));
        assert!(output.contains("state"));
        assert!(output.contains("session"));
        assert!(output.contains("new"));
        assert!(output.contains("/status"));
        assert!(output.contains("/resume latest"));
    }

    #[test]
    fn repl_help_lists_status_command() {
        let output = format_repl_help();

        assert!(output.contains("/help"));
        assert!(output.contains("/status"));
        assert!(output.contains("/sessions"));
        assert!(output.contains("/resume latest"));
        assert!(output.contains("/exit"));
    }

    #[test]
    fn short_id_keeps_short_values_and_truncates_long_values() {
        assert_eq!(short_id("01ABC"), "01ABC");
        assert_eq!(short_id("01ARYZ6S41YYYYYYYYYYYYYYYY"), "01ARYZ6S41");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```powershell
cargo test interfaces::cli::ui --lib
```

Expected: tests fail because the helper returns empty strings and `ui` is not exported yet.

- [ ] **Step 3: Export the module**

Modify `src/interfaces/cli/mod.rs`:

```rust
pub mod approval;
pub mod args;
pub mod config;
pub mod index;
pub mod input;
pub mod oneshot;
pub mod render;
pub mod repl;
pub mod runtime;
pub mod sessions;
pub mod state;
pub mod ui;
```

Keep existing module lines in their current order if the file already differs; the important addition is `pub mod ui;`.

- [ ] **Step 4: Implement formatting helpers**

Replace the stub implementation in `src/interfaces/cli/ui.rs` with:

```rust
use std::path::Path;

use crate::config::AppConfig;
use crate::core::types::TaskState;
use crate::core::workspace::{Workspace, WorkspaceKind};

pub struct ReplStatusView<'a> {
    pub workspace: &'a Workspace,
    pub config: &'a AppConfig,
    pub model_id: &'a str,
    pub active_resume_state: Option<&'a TaskState>,
}

pub fn format_repl_status(view: ReplStatusView<'_>) -> String {
    let workspace_kind = workspace_kind_label(view.workspace.kind);
    let workspace_root = view.workspace.root.to_string_lossy();
    let state = display_path(&view.workspace.state_dir, &view.workspace.root);
    let session = match view.active_resume_state {
        Some(state) => format!("resumed {}", short_id(state.run_id.to_string())),
        None => "new".to_string(),
    };

    format!(
        "\
rove
local-first agent runtime
workspace  {workspace_kind}  {workspace_root}
model      {model}
provider   {provider}
state      {state}  ·  session {session}

{commands}
",
        model = truncate_middle(view.model_id, 96),
        provider = view.config.provider.name,
        commands = command_hint_line(),
    )
}

pub fn format_repl_help() -> String {
    "\
Commands:
  /help             show this help
  /status           show workspace, model, provider, state, and session
  /exit, /quit      exit the REPL
  /clear            clear the terminal
  /sessions         list resumable task states
  /resume latest    resume the latest task state
  /resume <run_id>  resume a specific task state
"
    .to_string()
}

pub fn command_hint_line() -> &'static str {
    "/help  /sessions  /resume latest  /status  /clear  /exit"
}

pub fn short_id(value: impl AsRef<str>) -> String {
    let value = value.as_ref();
    if value.len() <= 10 {
        value.to_string()
    } else {
        value.chars().take(10).collect()
    }
}

pub fn truncate_middle(value: &str, max_chars: usize) -> String {
    let total = value.chars().count();
    if total <= max_chars || max_chars < 8 {
        return value.to_string();
    }
    let left = (max_chars - 1) / 2;
    let right = max_chars - 1 - left;
    let prefix: String = value.chars().take(left).collect();
    let suffix: String = value
        .chars()
        .rev()
        .take(right)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{prefix}…{suffix}")
}

fn workspace_kind_label(kind: WorkspaceKind) -> &'static str {
    match kind {
        WorkspaceKind::Folder => "folder",
        WorkspaceKind::Repo => "repo",
        WorkspaceKind::Task => "task",
    }
}

fn display_path(path: &Path, workspace_root: &Path) -> String {
    path.strip_prefix(workspace_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
```

- [ ] **Step 5: Run helper tests**

Run:

```powershell
cargo test interfaces::cli::ui --lib
```

Expected: pass.

## Task 2: Add `/status` And Compact Startup Banner

**Files:**
- Modify: `src/interfaces/cli/repl.rs`
- Modify: `tests/cli_repl.rs`

- [ ] **Step 1: Add parser unit test for `/status`**

In `src/interfaces/cli/repl.rs`, update the test `slash_command_parser_recognizes_first_pass_commands` to include:

```rust
assert_eq!(SlashCommand::parse("/status"), SlashCommand::Status);
```

Add a new enum variant:

```rust
Status,
```

Expected compile failure until parser and match arms are updated.

- [ ] **Step 2: Add smoke test for `/status`**

Append this test to `tests/cli_repl.rs`:

```rust
#[test]
fn repl_status_command_prints_runtime_context() {
    let tmp = tempfile::TempDir::new().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rove"))
        .arg("--cwd")
        .arg(tmp.path())
        .arg("--model")
        .arg("fake")
        .arg("--approval")
        .arg("never")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write as _;
            child.stdin.as_mut().unwrap().write_all(b"/status\n/exit\n")?;
            child.wait_with_output()
        })
        .unwrap();

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("workspace"));
    assert!(stderr.contains("model"));
    assert!(stderr.contains("fake"));
    assert!(stderr.contains("provider"));
    assert!(stderr.contains("state"));
    assert!(stderr.contains("session new"));
}
```

- [ ] **Step 3: Update existing REPL smoke expectation**

In `tests/cli_repl.rs`, replace:

```rust
assert!(stderr.contains("rove REPL - type /help for commands, /exit to quit"));
```

with:

```rust
assert!(stderr.contains("local-first agent runtime"));
assert!(stderr.contains("/status"));
assert!(stderr.contains("rove>"));
```

If `rustyline` prompt does not appear in captured stderr on the local platform, keep only the first two assertions. Do not assert ANSI color codes.

- [ ] **Step 4: Run failing REPL tests**

Run:

```powershell
cargo test --test cli_repl
cargo test interfaces::cli::repl::tests::slash_command_parser_recognizes_first_pass_commands --lib
```

Expected: fail because `/status` is not parsed or handled yet, and startup still prints the old one-line banner.

- [ ] **Step 5: Implement `/status` and startup banner**

Modify imports in `src/interfaces/cli/repl.rs`:

```rust
use crate::interfaces::cli::ui::{format_repl_help, format_repl_status, ReplStatusView};
```

Add enum variant:

```rust
Status,
```

Update `SlashCommand::parse`:

```rust
"/status" => Self::Status,
```

Replace the startup banner in `run`:

```rust
eprintln!("rove REPL - type /help for commands, /exit to quit");
```

with:

```rust
let mut state = ReplState::new(SessionId::new());
eprintln!(
    "{}",
    format_repl_status(ReplStatusView {
        workspace: &runtime.workspace,
        config: &runtime.config,
        model_id: runtime.engine.model_id(),
        active_resume_state: state.active_resume_state(),
    })
);
```

Move the existing `let mut state = ReplState::new(SessionId::new());` so it is not declared twice.

Update `handle_slash_command`:

```rust
SlashCommand::Help => {
    eprint!("{}", format_repl_help());
}
SlashCommand::Status => {
    eprintln!(
        "{}",
        format_repl_status(ReplStatusView {
            workspace: &runtime.workspace,
            config: &runtime.config,
            model_id: runtime.engine.model_id(),
            active_resume_state: state.active_resume_state(),
        })
    );
}
```

Remove or stop using the old `print_help()` function. If keeping it temporarily, ensure it delegates to `format_repl_help()` and includes `/status`.

- [ ] **Step 6: Run REPL tests**

Run:

```powershell
cargo test --test cli_repl
cargo test interfaces::cli::repl --lib
```

Expected: pass.

## Task 3: Add Explicit Render Modes

**Files:**
- Modify: `src/interfaces/cli/render.rs`
- Modify: `src/interfaces/cli/oneshot.rs`
- Modify: `src/interfaces/cli/repl.rs`

- [ ] **Step 1: Add render mode tests**

In `src/interfaces/cli/render.rs` tests, add:

```rust
#[tokio::test]
async fn repl_compact_render_prints_terminal_reason() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let state_store = StateStore::new(&workspace.state_dir);
    let run = state_store
        .start_run(SessionId::new(), JobId::new(), RunId::new())
        .unwrap();
    let events = stream::iter(vec![
        StreamEvent::RunStarted {
            run_id: run.run_id,
            job_id: run.job_id,
            user_message: "hello".to_string(),
        },
        StreamEvent::RunCompleted {
            reason: TerminationReason::Final,
            output: Some("hi".to_string()),
        },
    ]);

    let reason = render_run_events(
        events,
        CliRunRenderContext {
            message: "hello".to_string(),
            run,
            resume_state: None,
            state_store: &state_store,
            workspace: &workspace,
            model_id: "fake",
        },
        CliRunRenderOptions {
            mode: CliRunRenderMode::ReplCompact,
            print_done_line: true,
            print_trailing_newline: true,
        },
    )
    .await;

    assert_eq!(reason, TerminationReason::Final);
}
```

This is intentionally light because direct stdout/stderr capture inside async unit tests is brittle. The compile-time use of `CliRunRenderMode::ReplCompact` will fail until the mode exists.

- [ ] **Step 2: Run failing render test**

Run:

```powershell
cargo test interfaces::cli::render::tests::repl_compact_render_prints_terminal_reason --lib
```

Expected: fail to compile because `CliRunRenderMode` and the new option field do not exist.

- [ ] **Step 3: Add render mode types**

Modify `src/interfaces/cli/render.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliRunRenderMode {
    OneShot,
    ReplCompact,
}

#[derive(Debug, Clone, Copy)]
pub struct CliRunRenderOptions {
    pub mode: CliRunRenderMode,
    pub print_done_line: bool,
    pub print_trailing_newline: bool,
}

impl Default for CliRunRenderOptions {
    fn default() -> Self {
        Self {
            mode: CliRunRenderMode::OneShot,
            print_done_line: true,
            print_trailing_newline: true,
        }
    }
}
```

Update any existing struct initializers in tests to include `mode` or use `..Default::default()`.

- [ ] **Step 4: Pass explicit modes from one-shot and REPL**

In `src/interfaces/cli/oneshot.rs`, import `CliRunRenderMode` and call:

```rust
CliRunRenderOptions {
    mode: CliRunRenderMode::OneShot,
    ..CliRunRenderOptions::default()
}
```

In `src/interfaces/cli/repl.rs`, import `CliRunRenderMode` and call:

```rust
CliRunRenderOptions {
    mode: CliRunRenderMode::ReplCompact,
    ..CliRunRenderOptions::default()
}
```

- [ ] **Step 5: Run renderer and CLI tests**

Run:

```powershell
cargo test interfaces::cli::render --lib
cargo test --test cli_repl
```

Expected: pass, but output shape is not yet improved beyond mode plumbing.

## Task 4: Implement Compact REPL Event Rendering

**Files:**
- Modify: `src/interfaces/cli/render.rs`
- Test: `tests/cli_repl.rs`

- [ ] **Step 1: Add an end-to-end REPL run output test**

Append to `tests/cli_repl.rs`:

```rust
#[test]
fn repl_fake_run_uses_compact_sections() {
    let tmp = tempfile::TempDir::new().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rove"))
        .arg("--cwd")
        .arg(tmp.path())
        .arg("--model")
        .arg("fake")
        .arg("--approval")
        .arg("never")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write as _;
            child.stdin.as_mut().unwrap().write_all(b"hello\n/exit\n")?;
            child.wait_with_output()
        })
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(stderr.contains("You"));
    assert!(stderr.contains("hello"));
    assert!(stderr.contains("Done"));
    assert!(stderr.contains("final"));
    assert!(stderr.contains("report"));
    assert!(stdout.contains("fake response: hello"));
}
```

This fake run may not emit plan/tool events, so this test covers the common no-tool path. Tool/plan formatting is covered by unit tests in the next step.

- [ ] **Step 2: Add renderer unit test for plan/tool compact labels**

In `src/interfaces/cli/render.rs` tests, add a compile-and-run test that feeds plan/tool events. Use existing constructors/types from `crate::core::types` as needed. If `TaskPlan` or `ToolResult` field names differ, inspect `src/core/types.rs` and use the actual names.

The intended event vector is:

```rust
let call_id = crate::core::types::ToolCallId::new();
let events = stream::iter(vec![
    StreamEvent::RunStarted {
        run_id,
        job_id,
        user_message: "use echo".to_string(),
    },
    StreamEvent::PlanCreated {
        plan: crate::core::types::TaskPlan {
            goal: "use echo".to_string(),
            steps: vec![crate::core::types::PlanStep {
                id: "step-1".to_string(),
                title: "Run echo".to_string(),
                done: false,
            }],
            current_step: 0,
        },
    },
    StreamEvent::ToolCallStarted {
        call_id,
        tool_use_id: None,
        name: "echo".to_string(),
        args: serde_json::json!({"message":"hello"}),
    },
    StreamEvent::ToolCallCompleted {
        call_id,
        result: crate::core::types::ToolResult {
            call_id,
            output: "hello".to_string(),
            mutations: Vec::new(),
        },
    },
    StreamEvent::RunCompleted {
        reason: TerminationReason::Final,
        output: Some("hello".to_string()),
    },
]);
```

The test can assert only the returned reason if stdout capture is not practical. The important part is that compact rendering handles all event variants without panic.

- [ ] **Step 3: Implement render state counters**

Inside `render_run_events`, before the loop add:

```rust
let mut plan_step_count = 0usize;
let mut tool_call_count = 0usize;
let mut tool_failure_count = 0usize;
let mut printed_plan = false;
```

Add helpers near the bottom of `render.rs`:

```rust
fn is_repl(options: CliRunRenderOptions) -> bool {
    matches!(options.mode, CliRunRenderMode::ReplCompact)
}

fn relative_report_path(workspace: &Workspace, run_dir: &std::path::Path) -> String {
    run_dir
        .join("report.json")
        .strip_prefix(&workspace.root)
        .unwrap_or(&run_dir.join("report.json"))
        .to_string_lossy()
        .replace('\\', "/")
}

fn print_repl_block(label: &str, detail: impl AsRef<str>) {
    eprintln!("{label}");
    let detail = detail.as_ref();
    if !detail.is_empty() {
        for line in detail.lines() {
            eprintln!("  {line}");
        }
    }
}
```

If the borrow checker rejects `unwrap_or(&run_dir.join(...))`, store `let report_path = run_dir.join("report.json");` before `strip_prefix`.

- [ ] **Step 4: Implement compact event branches**

In the match:

For `RunStarted`:

```rust
StreamEvent::RunStarted { user_message, .. } if is_repl(options) => {
    print_repl_block("You", user_message);
}
```

For `PlanCreated`:

```rust
StreamEvent::PlanCreated { plan: new_plan } => {
    plan_step_count = new_plan.steps.len();
    printed_plan = true;
    if is_repl(options) {
        eprintln!("\nPlan · {} steps", new_plan.steps.len());
        for (index, step) in new_plan.steps.iter().enumerate() {
            eprintln!("  {}. {}", index + 1, step.title);
        }
    } else {
        eprintln!("\n  [plan] {} steps", new_plan.steps.len());
    }
}
```

For `PlanStepStarted`, skip in REPL if `printed_plan` is true:

```rust
StreamEvent::PlanStepStarted { step, index } => {
    if !is_repl(options) || !printed_plan {
        eprintln!("  [step {}] {}", index + 1, step.title);
    }
}
```

For `ToolCallStarted`:

```rust
StreamEvent::ToolCallStarted { name, args, .. } => {
    tool_call_count += 1;
    if is_repl(options) {
        eprintln!("\nTool · {name}");
        eprintln!("  {}", truncate(&args.to_string(), 200));
    } else {
        eprintln!("\n  [tool] {}({})", name, args);
    }
}
```

For `ToolCallCompleted`:

```rust
StreamEvent::ToolCallCompleted { result, .. } => {
    if is_repl(options) {
        eprintln!("  {}", truncate(&result.output, 200));
    } else {
        eprintln!("  [result] {}", truncate(&result.output, 200));
    }
}
```

For `ToolCallFailed`:

```rust
StreamEvent::ToolCallFailed { error, .. } => {
    tool_failure_count += 1;
    if is_repl(options) {
        eprintln!("\nError · tool");
        eprintln!("  {}", error);
    } else {
        eprintln!("  [error] {}", error);
    }
}
```

For `RunCompleted`, preserve final output behavior and add REPL summary:

```rust
StreamEvent::RunCompleted { reason, output } => {
    terminal_reason = reason.clone();
    if let Some(ref text) = output
        && !matches!(reason, TerminationReason::Final)
    {
        println!("\n{}", text);
    }
    if options.print_done_line {
        if is_repl(options) {
            eprintln!("\nDone · {:?}", reason);
            eprintln!(
                "  {} steps · {} tools · {} failures · report {}",
                plan_step_count,
                tool_call_count,
                tool_failure_count,
                relative_report_path(workspace, &run_dir)
            );
        } else {
            eprintln!("\n  [done] {:?}", reason);
        }
    }
    break;
}
```

- [ ] **Step 5: Run compact render tests**

Run:

```powershell
cargo test interfaces::cli::render --lib
cargo test --test cli_repl
```

Expected: pass.

## Task 5: Update Runtime Documentation

**Files:**
- Modify: `docs/runtime/implementation-guide.md`

- [ ] **Step 1: Update REPL documentation**

Find the section around "Running `rove` with no task enters the REPL" in `docs/runtime/implementation-guide.md`. Replace the old prompt-only snippet with:

````markdown
Running `rove` with no task enters the compact line-oriented REPL in the current
terminal. Startup prints the active workspace, model, provider, state directory,
session status, and common commands:

```text
rove
local-first agent runtime
workspace  repo  <workspace-root>
model      <model-id>
provider   <provider>
state      .rove  ·  session new

/help  /sessions  /resume latest  /status  /clear  /exit
rove>
```

The REPL remains a normal terminal prompt, not a full-screen TUI. During runs it
prints compact `You`, `Plan`, `Tool`, `Error`, and `Done` sections, while the
Web workbench remains the richer report/history surface.
````

Be careful with nested code fences in Markdown. If the surrounding section is already inside a fence, close it first.

- [ ] **Step 2: Add `/status` to the command table**

In the REPL command table, add:

```markdown
| `/status` | Print workspace, model, provider, state directory, and session status. |
```

- [ ] **Step 3: Run docs-related tests**

Run:

```powershell
cargo test --test code_hygiene
```

Expected: pass.

## Task 6: Final Verification And Commit

**Files:**
- All modified files above.

- [ ] **Step 1: Run formatting**

Run:

```powershell
cargo fmt --all --check
```

Expected: exit 0.

- [ ] **Step 2: Run linting**

Run:

```powershell
cargo clippy --all-targets -- -D warnings
```

Expected: exit 0.

- [ ] **Step 3: Run focused tests**

Run:

```powershell
cargo test interfaces::cli::ui --lib
cargo test interfaces::cli::repl --lib
cargo test interfaces::cli::render --lib
cargo test --test cli_repl
cargo test --test code_hygiene
```

Expected: all pass.

- [ ] **Step 4: Run full Rust tests**

Run:

```powershell
cargo test
```

Expected: all pass.

- [ ] **Step 5: Check worktree**

Run:

```powershell
git status --short
```

Expected: only intentional files are modified. Do not commit local browser
mockup artifacts.

- [ ] **Step 6: Commit**

Run:

```powershell
git add src/interfaces/cli/ui.rs src/interfaces/cli/mod.rs src/interfaces/cli/repl.rs src/interfaces/cli/render.rs src/interfaces/cli/oneshot.rs tests/cli_repl.rs docs/runtime/implementation-guide.md
git commit -m "Improve compact REPL presentation"
```

Expected: commit succeeds.

## Implementation Notes

- Keep stdout behavior for assistant/final text as close as possible to current one-shot behavior.
- Use stderr for REPL status/event metadata so stdout remains useful for content.
- Do not add `ratatui`, `crossterm`, or full-screen terminal dependencies in this pass.
- Do not commit local visual mockup artifacts from the design discussion.
- If tests that capture `rustyline` prompts behave differently across platforms, assert stable banner/status text instead of the prompt itself.
