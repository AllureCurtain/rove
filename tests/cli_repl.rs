use std::path::PathBuf;
use std::process::{Command, Stdio};

fn workspace_root() -> PathBuf {
    let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    root.pop();
    root
}

fn rove_bin() -> PathBuf {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_rove") {
        return PathBuf::from(path);
    }
    let root = workspace_root();
    let exe = if cfg!(windows) { "rove.exe" } else { "rove" };
    let candidate = root.join("target/debug").join(exe);
    if !candidate.exists() {
        let status = Command::new(env!("CARGO"))
            .args(["build", "-p", "rove-cli", "--bin", "rove"])
            .current_dir(&root)
            .status()
            .expect("failed to spawn cargo build for rove-cli");
        assert!(
            status.success(),
            "cargo build -p rove-cli --bin rove failed"
        );
    }
    assert!(
        candidate.exists(),
        "expected built CLI binary at {}",
        candidate.display()
    );
    candidate
}

#[test]
fn no_args_accepts_exit_command_and_exits_zero() {
    let tmp = tempfile::TempDir::new().unwrap();
    let output = Command::new(rove_bin())
        .arg("--cwd")
        .arg(tmp.path())
        .arg("--model")
        .arg("fake")
        .arg("--approval")
        .arg("never")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write as _;
            child.stdin.as_mut().unwrap().write_all(b"/exit\n")?;
            child.wait_with_output()
        })
        .unwrap();

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("R O V E"));
    assert!(stderr.contains("local-first agent runtime"));
    assert!(stderr.contains("model   fake"));
    assert!(stderr.contains("session  new"));
    assert!(stderr.contains("mode    interactive"));
    assert!(stderr.contains("status   ready"));
    assert!(stderr.contains("Type your task, or use /help for commands."));
    assert!(!stderr.contains("provider"));
    assert!(!stderr.contains("session id"));
    assert!(!stderr.contains("memory"));
    assert!(tmp.path().join(".rove").join("repl_history").exists());
}

#[test]
fn repl_status_command_prints_runtime_context() {
    let tmp = tempfile::TempDir::new().unwrap();
    let output = Command::new(rove_bin())
        .arg("--cwd")
        .arg(tmp.path())
        .arg("--model")
        .arg("fake")
        .arg("--approval")
        .arg("never")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write as _;
            child
                .stdin
                .as_mut()
                .unwrap()
                .write_all(b"/status\n/exit\n")?;
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
    assert!(stderr.contains("session id"));
    assert!(stderr.contains("active"));
    assert!(stderr.contains("memory"));
    assert!(stderr.contains(".rove/memory/sessions"));
}

#[test]
fn repl_fake_run_uses_compact_sections() {
    let tmp = tempfile::TempDir::new().unwrap();
    let output = Command::new(rove_bin())
        .arg("--cwd")
        .arg(tmp.path())
        .arg("--model")
        .arg("fake")
        .arg("--approval")
        .arg("never")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
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
    assert!(stderr.contains("Assistant"));
    assert!(stderr.contains("final · 1 step"));
    assert!(!stderr.contains("INFO"));
    assert!(!stderr.contains("Workspace detected"));
    assert!(!stderr.contains("Plan · 1 steps"));
    assert!(stdout.contains("fake response: hello"));
    assert!(!stdout.contains("INFO"));
    assert!(!stdout.contains("Workspace detected"));
}

#[test]
fn message_enters_repl_runs_first_prompt_and_accepts_exit() {
    let tmp = tempfile::TempDir::new().unwrap();
    let output = Command::new(rove_bin())
        .arg("--cwd")
        .arg(tmp.path())
        .arg("--model")
        .arg("fake")
        .arg("--approval")
        .arg("never")
        .arg("hello")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
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

#[test]
fn unquoted_multi_word_message_enters_repl_as_initial_prompt() {
    let tmp = tempfile::TempDir::new().unwrap();
    let output = Command::new(rove_bin())
        .arg("--cwd")
        .arg(tmp.path())
        .arg("--model")
        .arg("fake")
        .arg("--approval")
        .arg("never")
        .args(["hello", "world"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
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

#[test]
fn exec_message_does_not_wait_for_repl_input() {
    let tmp = tempfile::TempDir::new().unwrap();
    let output = Command::new(rove_bin())
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
    let output = Command::new(rove_bin())
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
