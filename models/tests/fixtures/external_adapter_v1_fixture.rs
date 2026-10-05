//! Deterministic external-adapter-v1 fixture used by unit tests.
//!
//! Modes (argv[1]):
//! - happy: text + usage + done
//! - malformed: emit non-JSON
//! - hang: hello_ok then silence
//! - secret: require secrets.primary, reply secret-present

use std::io::{self, BufRead, Write};

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "happy".to_string());
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut lines = stdin.lock().lines();

    let hello = lines.next().and_then(Result::ok).unwrap_or_default();
    let _ = hello;
    writeln!(
        stdout,
        r#"{{"type":"hello_ok","protocol":"external-adapter-v1","version":1}}"#
    )
    .unwrap();
    stdout.flush().unwrap();

    match mode.as_str() {
        "hang" => {
            // Keep the process alive without writing further events.
            loop {
                std::thread::sleep(std::time::Duration::from_secs(30));
            }
        }
        "malformed" => {
            let _request = lines.next().and_then(Result::ok).unwrap_or_default();
            writeln!(stdout, "not-json SECRET-VALUE").unwrap();
            stdout.flush().unwrap();
        }
        "secret" => {
            let request = lines.next().and_then(Result::ok).unwrap_or_default();
            let has_secret = request.contains("SECRET-VALUE");
            if has_secret {
                writeln!(
                    stdout,
                    r#"{{"type":"text_delta","text":"secret-present"}}"#
                )
                .unwrap();
            } else {
                writeln!(
                    stdout,
                    r#"{{"type":"error","code":"auth_failed","message":"missing"}}"#
                )
                .unwrap();
            }
            writeln!(
                stdout,
                r#"{{"type":"usage","prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#
            )
            .unwrap();
            writeln!(stdout, r#"{{"type":"done"}}"#).unwrap();
            stdout.flush().unwrap();
        }
        _ => {
            let _request = lines.next().and_then(Result::ok).unwrap_or_default();
            writeln!(
                stdout,
                r#"{{"type":"text_delta","text":"adapter-ok"}}"#
            )
            .unwrap();
            writeln!(
                stdout,
                r#"{{"type":"usage","prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#
            )
            .unwrap();
            writeln!(stdout, r#"{{"type":"done"}}"#).unwrap();
            stdout.flush().unwrap();
        }
    }
}
