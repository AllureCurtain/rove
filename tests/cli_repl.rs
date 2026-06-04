#[test]
fn no_args_accepts_exit_command_and_exits_zero() {
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
            child.stdin.as_mut().unwrap().write_all(b"/exit\n")?;
            child.wait_with_output()
        })
        .unwrap();

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("local-first agent runtime"));
    assert!(stderr.contains("/status"));
    assert!(tmp.path().join(".rove").join("repl_history").exists());
}

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
fn one_shot_message_does_not_wait_for_repl_input() {
    let tmp = tempfile::TempDir::new().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rove"))
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
    assert!(!stdout.contains("INFO"));
    assert!(!stdout.contains("Workspace detected"));
    assert!(!stderr.contains("INFO"));
    assert!(!stderr.contains("Workspace detected"));
}

#[test]
fn unquoted_multi_word_one_shot_joins_message() {
    let tmp = tempfile::TempDir::new().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rove"))
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
