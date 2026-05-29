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
    assert!(stderr.contains("rove REPL - type /help for commands, /exit to quit"));
    assert!(tmp.path().join(".rove").join("repl_history").exists());
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
    assert!(String::from_utf8_lossy(&output.stdout).contains("fake response: hello"));
}
