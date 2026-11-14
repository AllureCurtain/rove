//! Helpers shared by modules of the `it` test binary.

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

/// Path to a freshly built `rove` CLI binary.
///
/// Cargo only sets `CARGO_BIN_EXE_rove` for tests inside the package that owns
/// the binary, so this package builds it once per test process. The binary is
/// located next to the running test executable (`<target>/debug/deps/..`), which
/// stays correct when `CARGO_TARGET_DIR` moves the target directory.
pub fn rove_bin() -> PathBuf {
    static ROVE_BIN: OnceLock<PathBuf> = OnceLock::new();
    ROVE_BIN
        .get_or_init(|| {
            if let Ok(path) = std::env::var("CARGO_BIN_EXE_rove") {
                return PathBuf::from(path);
            }
            // Ask cargo for the binary path rather than deriving it from the
            // test executable's location. Deriving `target/<profile>` from
            // `current_exe` breaks when the runner places test binaries under
            // `target/<profile>/build/<pkg>/<hash>/` (shared/scratch target
            // layouts), because the deps parent is no longer the profile dir.
            let output = Command::new(env!("CARGO"))
                .args([
                    "build",
                    "-p",
                    "rove-cli",
                    "--bin",
                    "rove",
                    "--message-format=json-render-diagnostics",
                ])
                .current_dir(workspace_root())
                .output()
                .expect("failed to spawn cargo build for rove-cli");
            assert!(
                output.status.success(),
                "cargo build -p rove-cli --bin rove failed:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let exe = stdout_bin(&output.stdout).unwrap_or_else(|| {
                panic!(
                    "cargo build produced no `rove` executable artifact:\n{}",
                    String::from_utf8_lossy(&output.stdout)
                )
            });
            assert!(
                exe.exists(),
                "expected built CLI binary at {}",
                exe.display()
            );
            exe
        })
        .clone()
}

/// Pull the `rove` executable path out of `cargo build --message-format=json`.
fn stdout_bin(stdout: &[u8]) -> Option<PathBuf> {
    for line in String::from_utf8_lossy(stdout).lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            continue;
        };
        if value.get("reason").and_then(|r| r.as_str()) != Some("compiler-artifact") {
            continue;
        }
        let matches_rove = value
            .get("target")
            .and_then(|t| t.get("name"))
            .and_then(|n| n.as_str())
            == Some("rove");
        if matches_rove {
            if let Some(exe) = value.get("executable").and_then(|e| e.as_str()) {
                return Some(PathBuf::from(exe));
            }
        }
    }
    None
}

pub fn workspace_root() -> PathBuf {
    let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    root.pop();
    root.pop();
    root
}
