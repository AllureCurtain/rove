# Rich Terminal REPL Implementation Plan

> **For implementers:** Execute this plan task by task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `rove` and `rove "prompt"` enter the rich interactive terminal REPL, and move non-interactive one-shot execution to `rove exec "prompt"`.

**Architecture:** Keep the existing runtime core untouched. Change CLI parsing/routing at `src/interfaces/cli/args.rs` and `src/main.rs`, adapt `src/interfaces/cli/repl.rs` to accept an initial prompt, wrap the existing one-shot backend behind an `exec` module, and refine the scrollback renderer in `src/interfaces/cli/render.rs`.

**Tech Stack:** Rust 2024, clap derive, tokio, rustyline, existing `StreamEvent`, existing terminal `RunViewUpdate`/`RunViewState`, existing cargo test stack.

---

## Starting Context

Work in this worktree:

```powershell
D:\Study\project\agent\rove\.worktrees\rich-terminal-repl-design
```

Branch:

```text
feature/rich-terminal-repl-design
```

Approved design spec:

```text
docs/design/2026-06-09-rich-terminal-repl-design.md
```

Baseline already verified before writing this plan:

```powershell
cargo build
cargo test
```

Both passed on the worktree before implementation planning.

## File Structure

- Modify `src/interfaces/cli/args.rs`: add the `exec` subcommand, make runtime flags usable after subcommands, and update parser tests.
- Modify `src/main.rs`: route bare messages to interactive REPL as initial prompts, and route `Command::Exec` to the non-interactive backend.
- Create `src/interfaces/cli/exec.rs`: product-named wrapper around the existing one-shot implementation.
- Modify `src/interfaces/cli/mod.rs`: export the new `exec` module.
- Modify `src/interfaces/cli/repl.rs`: accept `Option<String>` initial prompt and submit it before the readline loop.
- Modify `src/interfaces/cli/render.rs`: render shell tool calls as `Command` blocks in REPL mode and add small pure helpers for testable block decisions.
- Modify `src/interfaces/cli/ui.rs`: update REPL copy from compact "repl" wording to rich "interactive" wording.
- Modify `tests/cli_repl.rs`: replace old bare-message one-shot tests with interactive initial-prompt tests and explicit `exec` tests.
- Modify `README.md`: document interactive default and `exec`.
- Modify `docs/runtime/implementation-guide.md`: update the CLI startup path and smoke commands.

Do not introduce ratatui, crossterm, alternate-screen rendering, mouse support, or a fullscreen app shell.

---

### Task 1: CLI Parser Split

**Files:**
- Modify: `src/interfaces/cli/args.rs`

- [ ] **Step 1: Add failing parser tests**

In `src/interfaces/cli/args.rs`, update the existing tests near the bottom of the file.

Rename these test functions:

```rust
#[test]
fn quoted_task_still_parses_as_one_shot_message() {
    let args = Args::parse_from(["rove", "analyze this project"]);

    assert_eq!(args.message().as_deref(), Some("analyze this project"));
    assert!(args.command.is_none());
}

#[test]
fn unquoted_multi_word_task_parses_as_one_shot_message() {
    let args = Args::try_parse_from(["rove", "analyze", "this", "project"]).unwrap();

    assert_eq!(args.message().as_deref(), Some("analyze this project"));
    assert!(args.command.is_none());
}
```

to:

```rust
#[test]
fn quoted_task_parses_as_initial_prompt() {
    let args = Args::parse_from(["rove", "analyze this project"]);

    assert_eq!(args.message().as_deref(), Some("analyze this project"));
    assert!(args.command.is_none());
}

#[test]
fn unquoted_multi_word_task_parses_as_initial_prompt() {
    let args = Args::try_parse_from(["rove", "analyze", "this", "project"]).unwrap();

    assert_eq!(args.message().as_deref(), Some("analyze this project"));
    assert!(args.command.is_none());
}
```

Add these tests after `unquoted_multi_word_task_parses_as_initial_prompt`:

```rust
#[test]
fn exec_subcommand_parses_noninteractive_message() {
    let args = Args::parse_from(["rove", "exec", "analyze this project"]);

    assert!(args.message().is_none());
    match args.command {
        Some(Command::Exec { message }) => {
            assert_eq!(message, vec!["analyze this project".to_string()]);
        }
        other => panic!("expected exec subcommand, got {other:?}"),
    }
}

#[test]
fn exec_subcommand_joins_unquoted_multi_word_message() {
    let args = Args::parse_from(["rove", "exec", "analyze", "this", "project"]);

    assert!(args.message().is_none());
    match args.command {
        Some(Command::Exec { message }) => {
            assert_eq!(
                message.join(" "),
                "analyze this project".to_string()
            );
        }
        other => panic!("expected exec subcommand, got {other:?}"),
    }
}

#[test]
fn exec_subcommand_requires_message() {
    let err = Args::try_parse_from(["rove", "exec"]).unwrap_err();

    assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
}

#[test]
fn runtime_flags_parse_after_exec_subcommand() {
    let args = Args::parse_from([
        "rove",
        "exec",
        "--model",
        "fake",
        "--approval",
        "never",
        "hello",
    ]);

    assert_eq!(args.model.as_deref(), Some("fake"));
    assert!(matches!(args.approval, CliApprovalPolicy::Never));
    match args.command {
        Some(Command::Exec { message }) => assert_eq!(message, vec!["hello".to_string()]),
        other => panic!("expected exec subcommand, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run the parser tests and verify they fail**

Run:

```powershell
cargo test interfaces::cli::args::tests::exec_subcommand_parses_noninteractive_message --lib
```

Expected: fail to compile or fail at runtime because `Command::Exec` does not exist yet.

- [ ] **Step 3: Add `exec` parsing and global runtime flags**

In `src/interfaces/cli/args.rs`, add `global = true` to runtime-level flags that must work after `exec`:

```rust
#[arg(short, long, global = true)]
pub model: Option<String>,

#[arg(long, global = true)]
pub max_steps: Option<u32>,

#[arg(long, global = true)]
pub resume: Option<String>,

#[arg(long, value_enum, default_value_t = CliApprovalPolicy::Ask, global = true)]
pub approval: CliApprovalPolicy,

#[arg(short = 'C', long, global = true)]
pub cwd: Option<String>,

#[arg(long, global = true)]
pub task_workspace: Option<String>,

#[arg(long, global = true)]
pub task_base: Option<PathBuf>,
```

Add this variant to `Command` after `DumpConfig`:

```rust
/// Run a prompt non-interactively and exit.
Exec {
    /// The task or question to give the agent.
    #[arg(value_name = "MESSAGE", num_args = 1.., required = true)]
    message: Vec<String>,
},
```

Update imports in the test module only if the compiler requests them. The existing test module already imports `Args`, `CliApprovalPolicy`, and `Command`.

- [ ] **Step 4: Run parser tests and verify they pass**

Run:

```powershell
cargo test interfaces::cli::args::tests --lib
```

Expected: all `interfaces::cli::args::tests` pass.

- [ ] **Step 5: Commit parser split**

Run:

```powershell
git add src/interfaces/cli/args.rs
git commit -m "feat: parse exec command"
```

---

### Task 2: Route Exec And Interactive Initial Prompts

**Files:**
- Create: `src/interfaces/cli/exec.rs`
- Modify: `src/interfaces/cli/mod.rs`
- Modify: `src/main.rs`
- Modify: `tests/cli_repl.rs`

- [ ] **Step 1: Add failing CLI integration tests**

In `tests/cli_repl.rs`, replace `one_shot_message_does_not_wait_for_repl_input` with:

```rust
#[test]
fn message_enters_repl_runs_first_prompt_and_accepts_exit() {
    let tmp = tempfile::TempDir::new().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rove"))
        .arg("--cwd")
        .arg(tmp.path())
        .arg("--model")
        .arg("fake")
        .arg("--approval")
        .arg("never")
        .arg("hello")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write as _;
            child.stdin.as_mut().unwrap().write_all(b"/exit\n")?;
            child.wait_with_output()
        })
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("fake response: hello"));
    assert!(stderr.contains("R O V E") || stderr.contains("Rove"));
    assert!(stderr.contains("You"));
    assert!(stderr.contains("hello"));
    assert!(stderr.contains("Assistant"));
    assert!(stderr.contains("Done"));
    assert!(!stderr.contains("unexpected argument"));
}
```

Replace `unquoted_multi_word_one_shot_joins_message` with:

```rust
#[test]
fn unquoted_multi_word_message_enters_repl_as_initial_prompt() {
    let tmp = tempfile::TempDir::new().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rove"))
        .arg("--cwd")
        .arg(tmp.path())
        .arg("--model")
        .arg("fake")
        .arg("--approval")
        .arg("never")
        .args(["hello", "world"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write as _;
            child.stdin.as_mut().unwrap().write_all(b"/exit\n")?;
            child.wait_with_output()
        })
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("fake response: hello world"));
    assert!(stderr.contains("You"));
    assert!(stderr.contains("hello world"));
    assert!(!stderr.contains("unexpected argument"));
}
```

Add these two tests after the initial-prompt tests:

```rust
#[test]
fn exec_message_does_not_wait_for_repl_input() {
    let tmp = tempfile::TempDir::new().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rove"))
        .arg("exec")
        .arg("--cwd")
        .arg(tmp.path())
        .arg("--model")
        .arg("fake")
        .arg("--approval")
        .arg("never")
        .arg("hello")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("fake response: hello"));
    assert!(!stderr.contains("R O V E"));
    assert!(!stderr.contains("Rove"));
    assert!(!stderr.contains("mode    repl"));
    assert!(!stderr.contains("mode       interactive"));
}

#[test]
fn exec_unquoted_multi_word_message_joins_message() {
    let tmp = tempfile::TempDir::new().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rove"))
        .arg("exec")
        .arg("--cwd")
        .arg(tmp.path())
        .arg("--model")
        .arg("fake")
        .arg("--approval")
        .arg("never")
        .args(["hello", "world"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("fake response: hello world"));
    assert!(!stderr.contains("unexpected argument"));
}
```

- [ ] **Step 2: Run the new integration test and verify it fails**

Run:

```powershell
cargo test --test cli_repl exec_message_does_not_wait_for_repl_input
```

Expected: fail because `rove exec` is parsed but `main.rs` does not route `Command::Exec` yet.

- [ ] **Step 3: Add the product-named exec wrapper**

Create `src/interfaces/cli/exec.rs`:

```rust
use tokio_util::sync::CancellationToken;

use crate::core::engine::Engine;
use crate::core::types::{TaskState, TerminationReason};
use crate::interfaces::cli::oneshot::run_oneshot_with_cancel;
use crate::state::store::{RunHandle, StateStore};

/// Run a non-interactive exec prompt: stream output, write artifacts, and exit.
pub async fn run_exec_with_cancel(
    engine: &Engine,
    message: String,
    run: RunHandle,
    resume_state: Option<TaskState>,
    state_store: &StateStore,
    cancel: CancellationToken,
) -> TerminationReason {
    run_oneshot_with_cancel(engine, message, run, resume_state, state_store, cancel).await
}
```

In `src/interfaces/cli/mod.rs`, add:

```rust
pub mod exec;
```

- [ ] **Step 4: Route `Command::Exec` in `main.rs`**

In `src/main.rs`, replace:

```rust
use rove::interfaces::cli::oneshot::run_oneshot_with_cancel;
```

with:

```rust
use rove::interfaces::cli::exec::run_exec_with_cancel;
```

In `async_main`, use a cloned command for subcommand routing:

```rust
async fn async_main(args: Args) -> anyhow::Result<()> {
    match args.command.clone() {
        Some(Command::Index {
            path,
            deterministic,
            embedding_model,
        }) => {
            return cli_index::run(IndexOptions {
                cwd: path.or_else(|| args.cwd.clone().map(PathBuf::from)),
                deterministic,
                embedding_model,
                eval_query: None,
                eval_kind: None,
                eval_limit: 8,
            })
            .await;
        }
        Some(Command::Sessions) => return sessions::run(args.cwd.clone()).await,
        Some(Command::State { command }) => return cli_state::run(args.cwd.clone(), command).await,
        Some(Command::Exec { message }) => {
            let message = join_message(message);
            let runtime = build_runtime(&args, Some(&message)).await?;
            return run_exec(args, runtime, message).await;
        }
        Some(Command::DumpConfig) => unreachable!("dump-config is handled before runtime startup"),
        None => {}
    }

    let message = args.message();
    let runtime = build_runtime(&args, message.as_ref()).await?;

    repl::run(runtime, message).await
}
```

Add these helpers below `async_main`:

```rust
async fn build_runtime(
    args: &Args,
    fake_message: Option<&String>,
) -> anyhow::Result<rove::interfaces::cli::runtime::CliRuntime> {
    build_cli_runtime(CliRuntimeOptions {
        cwd: args.cwd.clone().map(PathBuf::from),
        model: args.model.clone(),
        max_steps: args.max_steps,
        approval: args.approval,
        task_workspace: args.task_workspace.clone(),
        task_base: args.task_base.clone(),
        initial_fake_response: fake_message.map(|message| format!("fake response: {message}")),
    })
    .await
}

fn join_message(message: Vec<String>) -> String {
    message.join(" ").trim().to_string()
}
```

Rename the existing `run_oneshot` function to `run_exec` and update its body to call `run_exec_with_cancel`:

```rust
async fn run_exec(
    args: Args,
    runtime: rove::interfaces::cli::runtime::CliRuntime,
    message: String,
) -> anyhow::Result<()> {
    let resume_state = resolve_resume_state(&runtime.state_store, args.resume.as_deref()).await?;
    let run_id = RunId::new();
    let run_handle = runtime.state_store.start_run(
        resume_state
            .as_ref()
            .map(|state| state.session_id)
            .unwrap_or_default(),
        resume_state
            .as_ref()
            .map(|state| state.job_id)
            .unwrap_or_default(),
        run_id,
    )?;

    tracing::info!(%run_handle.run_id, "Starting exec run");

    let cli_cancel = CancellationToken::new();
    let signal_exit_code = spawn_cli_signal_listener(cli_cancel.clone());
    let termination = run_exec_with_cancel(
        &runtime.engine,
        message,
        run_handle,
        resume_state,
        &runtime.state_store,
        cli_cancel,
    )
    .await;
    if matches!(termination, TerminationReason::Cancelled) {
        std::process::exit(signal_exit_code.load(Ordering::SeqCst));
    }

    Ok(())
}
```

- [ ] **Step 5: Run the exec integration tests**

Run:

```powershell
cargo test --test cli_repl exec_message_does_not_wait_for_repl_input exec_unquoted_multi_word_message_joins_message
```

If this Cargo version rejects multiple test filters, run them separately:

```powershell
cargo test --test cli_repl exec_message_does_not_wait_for_repl_input
cargo test --test cli_repl exec_unquoted_multi_word_message_joins_message
```

Expected: both pass after routing is implemented.

- [ ] **Step 6: Commit exec routing**

Run:

```powershell
git add src/main.rs src/interfaces/cli/mod.rs src/interfaces/cli/exec.rs tests/cli_repl.rs
git commit -m "feat: route exec runs"
```

---

### Task 3: Initial Prompt Submission In REPL

**Files:**
- Modify: `src/interfaces/cli/repl.rs`
- Modify: `src/main.rs`
- Modify: `tests/cli_repl.rs`

- [ ] **Step 1: Run the initial prompt integration test and verify it fails**

Run:

```powershell
cargo test --test cli_repl message_enters_repl_runs_first_prompt_and_accepts_exit
```

Expected: fail to compile because `repl::run` still accepts only `CliRuntime`, or fail at runtime because the initial prompt is not auto-submitted.

- [ ] **Step 2: Change the REPL entry signature**

In `src/interfaces/cli/repl.rs`, change:

```rust
pub async fn run(runtime: CliRuntime) -> anyhow::Result<()> {
```

to:

```rust
pub async fn run(runtime: CliRuntime, initial_prompt: Option<String>) -> anyhow::Result<()> {
```

In `src/main.rs`, the call added in Task 2 must be:

```rust
repl::run(runtime, message).await
```

- [ ] **Step 3: Submit the initial prompt before the readline loop**

In `src/interfaces/cli/repl.rs`, after `load_history(&mut editor, &history_path);`, insert:

```rust
    if let Some(initial_prompt) = initial_prompt {
        let input = initial_prompt.trim();
        if !input.is_empty() {
            if let Err(err) = editor.add_history_entry(input) {
                eprintln!("warning: failed to record REPL history: {err}");
            }
            run_prompt(input.to_string(), &runtime, &mut state).await?;
            save_history(&mut editor, &history_path);
        }
    }
```

Leave slash-command handling inside the main readline loop. A quoted initial prompt such as `rove "/status"` is user text, not a slash command.

- [ ] **Step 4: Update no-argument REPL callers**

Find all calls to `repl::run(`:

```powershell
rg -n "repl::run\\(" src tests
```

Expected after Task 2 and this task: only `src/main.rs` calls it. If another caller exists, update it to pass `None`.

- [ ] **Step 5: Run REPL integration tests**

Run:

```powershell
cargo test --test cli_repl
```

Expected: all `tests/cli_repl.rs` tests pass. The old behavior where bare messages exit immediately is gone.

- [ ] **Step 6: Run focused lib tests**

Run:

```powershell
cargo test interfaces::cli::repl::tests --lib
```

Expected: all REPL unit tests pass.

- [ ] **Step 7: Commit initial prompt REPL behavior**

Run:

```powershell
git add src/main.rs src/interfaces/cli/repl.rs tests/cli_repl.rs
git commit -m "feat: submit initial prompts in repl"
```

---

### Task 4: Command Blocks In The Rich Renderer

**Files:**
- Modify: `src/interfaces/cli/render.rs`

- [ ] **Step 1: Add failing renderer helper tests**

In `src/interfaces/cli/render.rs`, inside the existing `#[cfg(test)] mod tests`, add these tests near `repl_update_labels_cover_pending_terminal_updates`:

```rust
#[test]
fn repl_tool_start_block_uses_command_for_shell_tool() {
    assert_eq!(
        super::repl_tool_start_block("shell", &serde_json::json!({"command":"cargo test"})),
        super::ReplToolStartBlock::Command {
            command: "cargo test".to_string()
        }
    );
    assert_eq!(
        super::repl_tool_start_block("fs_read", &serde_json::json!({"path":"README.md"})),
        super::ReplToolStartBlock::Tool {
            name: "fs_read".to_string(),
            args: "{\"path\":\"README.md\"}".to_string()
        }
    );
}

#[test]
fn shell_result_summary_formats_exit_code_and_streams() {
    let call_id = CallId::new();
    let result = ToolResult {
        call_id,
        output: serde_json::json!({
            "command": "cargo test",
            "success": true,
            "exit_code": 0,
            "stdout": "ok\n",
            "stderr": "",
            "stdout_truncated": false,
            "stderr_truncated": false
        })
        .to_string(),
        mutations: Vec::new(),
        metadata: Default::default(),
    };

    let lines = super::shell_result_summary(&result).unwrap();

    assert_eq!(lines[0], "exit 0");
    assert_eq!(lines[1], "stdout ok");
}

#[test]
fn shell_result_summary_marks_truncated_output() {
    let call_id = CallId::new();
    let result = ToolResult {
        call_id,
        output: serde_json::json!({
            "command": "cargo test",
            "success": false,
            "exit_code": 101,
            "stdout": "",
            "stderr": "failure\n",
            "stdout_truncated": false,
            "stderr_truncated": true
        })
        .to_string(),
        mutations: Vec::new(),
        metadata: Default::default(),
    };

    let lines = super::shell_result_summary(&result).unwrap();

    assert_eq!(lines[0], "exit 101");
    assert_eq!(lines[1], "stderr failure");
    assert_eq!(lines[2], "stderr truncated");
}
```

- [ ] **Step 2: Run helper tests and verify they fail**

Run:

```powershell
cargo test interfaces::cli::render::tests::repl_tool_start_block_uses_command_for_shell_tool --lib
```

Expected: fail to compile because `ReplToolStartBlock`, `repl_tool_start_block`, and `shell_result_summary` do not exist.

- [ ] **Step 3: Add renderer helper types and functions**

In `src/interfaces/cli/render.rs`, add this import at the top:

```rust
use serde::Deserialize;
```

Add these helpers above `render_repl_update`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
enum ReplToolStartBlock {
    Tool { name: String, args: String },
    Command { command: String },
}

#[derive(Debug, Deserialize)]
struct ShellOutputView {
    command: String,
    success: bool,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    stdout_truncated: bool,
    stderr_truncated: bool,
}

fn repl_tool_start_block(name: &str, args: &serde_json::Value) -> ReplToolStartBlock {
    if name == "shell"
        && let Some(command) = args.get("command").and_then(|value| value.as_str())
    {
        return ReplToolStartBlock::Command {
            command: command.to_string(),
        };
    }

    ReplToolStartBlock::Tool {
        name: name.to_string(),
        args: args.to_string(),
    }
}

fn shell_result_summary(result: &crate::core::types::ToolResult) -> Option<Vec<String>> {
    let output: ShellOutputView = serde_json::from_str(&result.output).ok()?;
    let mut lines = Vec::new();
    let exit = output
        .exit_code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    lines.push(format!("exit {exit}"));

    let stdout = output.stdout.trim();
    if !stdout.is_empty() {
        lines.push(format!("stdout {}", truncate(stdout, 200)));
    }

    let stderr = output.stderr.trim();
    if !stderr.is_empty() {
        lines.push(format!("stderr {}", truncate(stderr, 200)));
    }

    if output.stdout_truncated {
        lines.push("stdout truncated".to_string());
    }
    if output.stderr_truncated {
        lines.push("stderr truncated".to_string());
    }

    Some(lines)
}
```

If `cargo test` reports `dead_code` warnings for fields in `ShellOutputView`, remove unused `command` and `success` fields from the struct. Serde ignores extra JSON fields by default, so the view struct does not need to contain every field.

- [ ] **Step 4: Use `Command` blocks for shell starts**

In `render_repl_update`, replace the REPL branch for `RunViewUpdate::ToolCallStarted`:

```rust
if is_repl(options) {
    eprintln!("\nTool · {name}");
    eprintln!("  {}", truncate(&args.to_string(), 200));
} else {
    eprintln!("\n  [tool] {}({})", name, args);
}
```

with:

```rust
if is_repl(options) {
    match repl_tool_start_block(&name, &args) {
        ReplToolStartBlock::Command { command } => {
            eprintln!("\nCommand");
            eprintln!("  {command}");
        }
        ReplToolStartBlock::Tool { name, args } => {
            eprintln!("\nTool · {name}");
            eprintln!("  {}", truncate(&args, 200));
        }
    }
} else {
    eprintln!("\n  [tool] {}({})", name, args);
}
```

- [ ] **Step 5: Use shell result summaries for shell completions**

In `render_repl_update`, replace the `RunViewUpdate::ToolCallCompleted` arm:

```rust
RunViewUpdate::ToolCallCompleted { result, .. } => {
    if is_repl(options) {
        eprintln!("  {}", truncate(&result.output, 200));
    } else {
        eprintln!("  [result] {}", truncate(&result.output, 200));
    }
    None
}
```

with:

```rust
RunViewUpdate::ToolCallCompleted { call_id, result } => {
    if is_repl(options) {
        if render_state
            .tool_names
            .get(&call_id)
            .map(|name| name == "shell")
            .unwrap_or(false)
        {
            if let Some(lines) = shell_result_summary(&result) {
                for line in lines {
                    eprintln!("  {line}");
                }
            } else {
                eprintln!("  {}", truncate(&result.output, 200));
            }
        } else {
            eprintln!("  {}", truncate(&result.output, 200));
        }
    } else {
        eprintln!("  [result] {}", truncate(&result.output, 200));
    }
    None
}
```

- [ ] **Step 6: Run renderer tests**

Run:

```powershell
cargo test interfaces::cli::render::tests --lib
```

Expected: all renderer unit tests pass.

- [ ] **Step 7: Commit command block rendering**

Run:

```powershell
git add src/interfaces/cli/render.rs
git commit -m "feat: render shell tools as command blocks"
```

---

### Task 5: Terminal Copy And Documentation

**Files:**
- Modify: `src/interfaces/cli/ui.rs`
- Modify: `tests/cli_repl.rs`
- Modify: `README.md`
- Modify: `docs/runtime/implementation-guide.md`

- [ ] **Step 1: Add failing UI expectations**

In `src/interfaces/cli/ui.rs`, update tests that currently assert compact REPL wording.

In `repl_welcome_wide_layout_contains_compact_startup_context`, replace:

```rust
assert!(output.contains("local agent runtime"));
assert!(output.contains("mode    repl"));
```

with:

```rust
assert!(output.contains("local-first agent runtime"));
assert!(output.contains("mode    interactive"));
```

In `repl_welcome_narrow_layout_is_compact_and_bounded`, replace:

```rust
assert!(output.contains("mode repl  status ready"));
```

with:

```rust
assert!(output.contains("mode interactive  status ready"));
```

In `tests/cli_repl.rs`, update `no_args_accepts_exit_command_and_exits_zero`:

```rust
assert!(stderr.contains("local-first agent runtime"));
assert!(stderr.contains("mode    interactive"));
```

and remove or replace the old assertions for `"local agent runtime"` and `"mode    repl"`.

- [ ] **Step 2: Run UI tests and verify they fail**

Run:

```powershell
cargo test interfaces::cli::ui::tests --lib
```

Expected: fail because `format_repl_welcome` still prints `local agent runtime` and `mode repl`.

- [ ] **Step 3: Update welcome copy**

In `src/interfaces/cli/ui.rs`, in `format_repl_welcome`, replace:

```rust
  local agent runtime
```

with:

```rust
  local-first agent runtime
```

Replace:

```rust
  mode    repl{mode_pad}status   ready
```

with:

```rust
  mode    interactive{mode_pad}status   ready
```

Update `mode_pad` to account for the longer word:

```rust
mode_pad = " ".repeat(model_width.saturating_sub("interactive".len()) + 2),
```

In `format_compact_welcome`, replace:

```rust
local agent runtime
```

with:

```rust
local-first agent runtime
```

Replace:

```rust
mode repl  status ready
```

with:

```rust
mode interactive  status ready
```

If the narrow-width test fails because the line is too long, change the compact narrow line to:

```rust
interactive  ready
```

and assert that exact compact fallback in the narrow test instead of `mode interactive  status ready`.

- [ ] **Step 4: Update README quick start and entry point description**

In `README.md`, replace the "Quick Start" opening commands:

```markdown
Run a local fake-model task without network credentials:

```bash
cargo run -- --model fake "echo hello from rove"
```

Multi-word tasks can also be typed without shell quotes:

```bash
cargo run -- --model fake inspect this workspace
```
```

with:

````markdown
Start the interactive terminal REPL without network credentials:

```bash
cargo run -- --model fake
```

Start the same REPL with an initial prompt:

```bash
cargo run -- --model fake "echo hello from rove"
```

Run a non-interactive exec prompt and exit:

```bash
cargo run -- exec --model fake "echo hello from rove"
```
````

In the "Main Entry Points" table, replace the CLI purpose:

```markdown
| CLI | `src/main.rs` | One-shot task runs, config dump, sessions, and RAG indexing command dispatch. |
```

with:

```markdown
| CLI | `src/main.rs` | Rich terminal REPL, explicit `exec` runs, config dump, sessions, and RAG indexing command dispatch. |
```

- [ ] **Step 5: Update runtime implementation guide CLI section**

In `docs/runtime/implementation-guide.md`, in section `## 4. CLI Startup Path`, replace steps 13-15:

```markdown
13. Resolve optional CLI resume state.
14. If a message argument is present, run `run_oneshot_with_cancel`.
15. If no message and no subcommand are present, enter the line-oriented REPL.
```

with:

```markdown
13. Resolve optional CLI resume state when an exec run starts.
14. If `exec <message>` is present, run the non-interactive exec backend.
15. If a bare message argument is present, enter the rich terminal REPL and submit that message as the first prompt.
16. If no message and no subcommand are present, enter the rich terminal REPL and wait for input.
```

Replace:

```markdown
Current one-shot smoke command:

```powershell
cargo run -- --model fake "echo hello from rove"
```

The CLI also accepts unquoted multi-word tasks and joins them into the one-shot
message:

```powershell
cargo run -- --model fake inspect this workspace
```
```

with:

````markdown
Interactive REPL smoke command:

```powershell
cargo run -- --model fake
```

Interactive REPL with an initial prompt:

```powershell
cargo run -- --model fake "echo hello from rove"
```

Non-interactive exec smoke command:

```powershell
cargo run -- exec --model fake "echo hello from rove"
```

The CLI accepts unquoted multi-word initial prompts and exec prompts by joining
the trailing message words:

```powershell
cargo run -- --model fake inspect this workspace
cargo run -- exec --model fake inspect this workspace
```
````

Replace the sentence:

```markdown
Running `rove` with no task enters the compact line-oriented REPL in the current
terminal.
```

with:

```markdown
Running `rove` with no task enters the rich scrollback terminal REPL in the
current terminal.
```

- [ ] **Step 6: Run docs and UI checks**

Run:

```powershell
cargo test interfaces::cli::ui::tests --lib
cargo test --test cli_repl
cargo test --test code_hygiene runtime_docs_declare_current_mvp_boundary runtime_docs_record_phase_12_hygiene_and_source_of_truth_status
```

Expected: all pass. If `code_hygiene` rejects multiple filters in this Cargo version, run the two `code_hygiene` tests separately.

- [ ] **Step 7: Commit terminal copy and docs**

Run:

```powershell
git add src/interfaces/cli/ui.rs tests/cli_repl.rs README.md docs/runtime/implementation-guide.md
git commit -m "docs: document interactive terminal entry"
```

---

### Task 6: Full Verification And Final Review

**Files:**
- Check all modified files.

- [ ] **Step 1: Run formatting check**

Run:

```powershell
cargo fmt -- --check
```

Expected: pass with exit code 0.

If it fails, run:

```powershell
cargo fmt
git diff --check
```

Then commit formatting with the task that introduced the formatting drift:

```powershell
git add src README.md docs tests
git commit -m "style: format rich terminal repl changes"
```

- [ ] **Step 2: Run full test suite**

Run:

```powershell
cargo test
```

Expected: all tests pass.

- [ ] **Step 3: Run smoke commands manually**

Run:

```powershell
cargo run -- --model fake --approval never
```

Expected: prints the rich interactive welcome and waits at the `rove>` prompt. Type:

```text
/exit
```

Expected: exits with status 0.

Run:

```powershell
cargo run -- --model fake --approval never "hello from initial prompt"
```

Expected: prints welcome, runs the first prompt, prints `fake response: hello from initial prompt`, then waits at the REPL prompt. Type:

```text
/exit
```

Expected: exits with status 0.

Run:

```powershell
cargo run -- exec --model fake --approval never "hello from exec"
```

Expected: prints `fake response: hello from exec`, does not print the welcome block, and exits without waiting for `/exit`.

- [ ] **Step 4: Inspect git status and commit history**

Run:

```powershell
git status --short --branch
git log --oneline -6
```

Expected:

- Worktree is clean.
- Recent commits include:
  - `docs: design rich terminal repl`
  - `feat: parse exec command`
  - `feat: route exec runs`
  - `feat: submit initial prompts in repl`
  - `feat: render shell tools as command blocks`
  - `docs: document interactive terminal entry`

- [ ] **Step 5: Final implementation report**

Report these items:

- Worktree path.
- Branch name.
- Commit range from `main` to `HEAD`.
- Exact verification commands run and whether each passed.
- Any behavior intentionally not included. For this plan, `rove exec --json` is not implemented; the first pass only establishes `exec` as the non-interactive home for future script-specific output flags.

Do not merge back to `main` until the user asks.
