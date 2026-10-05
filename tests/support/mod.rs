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
            let status = Command::new(env!("CARGO"))
                .args(["build", "-p", "rove-cli", "--bin", "rove"])
                .current_dir(workspace_root())
                .status()
                .expect("failed to spawn cargo build for rove-cli");
            assert!(
                status.success(),
                "cargo build -p rove-cli --bin rove failed"
            );
            let profile_dir = std::env::current_exe()
                .expect("test executable path")
                .parent()
                .and_then(|deps| deps.parent())
                .expect("test executable lives in <target>/<profile>/deps")
                .to_path_buf();
            let exe = if cfg!(windows) { "rove.exe" } else { "rove" };
            let candidate = profile_dir.join(exe);
            assert!(
                candidate.exists(),
                "expected built CLI binary at {}",
                candidate.display()
            );
            candidate
        })
        .clone()
}

pub fn workspace_root() -> PathBuf {
    let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    root.pop();
    root
}
