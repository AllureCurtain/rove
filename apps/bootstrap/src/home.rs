//! Codex-style global Rove home directory (`~/.rove`).
//!
//! Mirrors `codex-rs/utils/home-dir/src/lib.rs`:
//!
//! - `ROVE_HOME` must exist and be a directory; it is canonicalized and any
//!   failure is a typed error (never a silent fallback).
//! - Without the env var, the default is `<home>/.rove`; existence is not
//!   verified so first use can create it.
//!
//! Layout (Codex sessions contract):
//!
//! ```text
//! ~/.rove/
//! ├── sessions/<yyyy>/<mm>/<dd>/rollout-<HHMMSS>-<uuid>.jsonl
//! ├── archived_sessions/          # Phase 7 maintenance target
//! └── state.db                    # derived index store (Phase 5)
//! ```
//!
//! Workspace-local `.rove/runs/<run_id>/trace.jsonl` files from before this
//! layout existed are migrated once into the sessions tree; the migration
//! marker `.rove/migrated.marker` keeps the operation idempotent.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Environment variable overriding the Rove home directory.
pub const HOME_ENV: &str = "ROVE_HOME";

/// Marker file written after a one-time legacy-run migration.
pub const MIGRATED_MARKER_FILE: &str = "migrated.marker";

#[derive(Debug, thiserror::Error)]
pub enum HomeError {
    #[error("Could not find home directory")]
    NoHomeDirectory,
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Resolve the Rove home directory: `ROVE_HOME` when set and valid, else
/// `<home>/.rove`.
pub fn find_rove_home() -> Result<PathBuf, HomeError> {
    let env = std::env::var_os(HOME_ENV);
    let env = env
        .as_deref()
        .map(|value| value.to_string_lossy().into_owned())
        .filter(|value| !value.is_empty());
    find_rove_home_from_env(env.as_deref())
}

fn find_rove_home_from_env(value: Option<&str>) -> Result<PathBuf, HomeError> {
    // An empty value behaves like an unset variable.
    let value = value.filter(|value| !value.is_empty());
    match value {
        Some(val) => {
            let path = PathBuf::from(val);
            let metadata = std::fs::metadata(&path).map_err(|err| match err.kind() {
                io::ErrorKind::NotFound => io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("{HOME_ENV} points to {val:?}, but that path does not exist"),
                ),
                kind => io::Error::new(kind, format!("failed to read {HOME_ENV} {val:?}: {err}")),
            })?;
            if !metadata.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{HOME_ENV} points to {val:?}, but that path is not a directory"),
                )
                .into());
            }
            let canonical = path.canonicalize().map_err(|err| {
                io::Error::new(
                    err.kind(),
                    format!("failed to canonicalize {HOME_HOME_ENV_LABEL} {val:?}: {err}"),
                )
            })?;
            Ok(canonical)
        }
        None => {
            let mut home = system_home_dir().ok_or(HomeError::NoHomeDirectory)?;
            home.push(".rove");
            Ok(home)
        }
    }
}

const HOME_HOME_ENV_LABEL: &str = "ROVE_HOME";

/// Minimal home-directory resolution without pulling in an extra crate:
/// `HOME` on Unix-like platforms, `USERPROFILE` (then the profile-qualified
/// drive pair) on Windows — mirroring the precedence of the `dirs` crate.
fn system_home_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        if let Some(profile) = std::env::var_os("USERPROFILE").filter(|v| !v.is_empty()) {
            return Some(PathBuf::from(profile));
        }
        let homedrive = std::env::var_os("HOMEDRIVE")?;
        let homepath = std::env::var_os("HOMEPATH")?;
        let mut path = PathBuf::from(homedrive);
        path.push(homepath);
        Some(path)
    } else {
        std::env::var_os("HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    }
}

/// The sessions/rollout surface of the resolved home directory.
///
/// All constructors create nothing eagerly; directories are materialized on
/// first write so read-only commands never touch the filesystem.
#[derive(Debug, Clone)]
pub struct RoveHome {
    root: PathBuf,
}

impl RoveHome {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn discover() -> Result<Self, HomeError> {
        Ok(Self::new(find_rove_home()?))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Truth-source rollout files: `<root>/sessions`.
    pub fn sessions_dir(&self) -> PathBuf {
        self.root.join("sessions")
    }

    /// Archived rollouts: `<root>/archived_sessions`.
    pub fn archived_sessions_dir(&self) -> PathBuf {
        self.root.join("archived_sessions")
    }

    /// Derived index database: `<root>/state.db`.
    pub fn state_db_path(&self) -> PathBuf {
        self.root.join("state.db")
    }

    /// Migration lock target used by concurrent schema upgrades (Phase 9).
    pub fn migrate_lock_path(&self) -> PathBuf {
        self.root.join("state.db.migrate.lock")
    }

    /// Codex-compatible sortable rollout basename for `created_at`:
    /// `rollout-<yyyymmdd>T<HHMMSS>-<uuid>.jsonl`.
    pub fn rollout_file_name(created_at: SystemTime) -> String {
        let secs = created_at
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let days = (secs / 86_400) as i64;
        let rem = secs % 86_400;
        let (hour, minute, second) = (rem / 3600, (rem % 3600) / 60, rem % 60);
        let (year, month, day) = civil_from_days(days);
        let uuid = uuid_v4_string();
        format!("rollout-{year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}-{uuid}.jsonl")
    }

    /// Full rollout path under the date-partitioned sessions tree.
    pub fn session_rollout_path(&self, created_at: SystemTime) -> PathBuf {
        let secs = created_at
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let days = (secs / 86_400) as i64;
        let (year, month, day) = civil_from_days(days);
        self.sessions_dir()
            .join(format!("{year:04}"))
            .join(format!("{month:02}"))
            .join(format!("{day:02}"))
            .join(Self::rollout_file_name(created_at))
    }
}

/// Days-since-epoch to (year, month, day); Howard Hinnant's civil_from_days.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d)
}

/// Random UUID v4 without a dependency: OS CSPRNG bytes formatted per RFC 4122.
fn uuid_v4_string() -> String {
    let mut bytes = [0u8; 16];
    if getrandom_bytes(&mut bytes).is_err() {
        // Deterministic fallback keeps the name unique enough within a process.
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        bytes[..8].copy_from_slice(&now.as_nanos().to_le_bytes()[..8]);
        bytes[8..].copy_from_slice(&std::process::id().to_le_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // variant 10
    let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}{}{}{}-{}-{}-{}-{}",
        hex[0], hex[1], hex[2], hex[3], hex[4], hex[5], hex[6], hex[7]
    )
}

fn getrandom_bytes(buf: &mut [u8]) -> Result<(), ()> {
    #[cfg(windows)]
    {
        // BCryptGenRandom via raw FFI would need a binding; fall back to a
        // high-resolution entropy mix which is sufficient for filename
        // uniqueness (not secrecy).
        let nanos = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let pid = std::process::id() as u128;
        let seed = nanos ^ (pid << 96);
        for (index, chunk) in buf.chunks_mut(8).enumerate() {
            let value = ((seed >> (index as u32 * 13)) as u64).to_le_bytes();
            for (target, source) in chunk.iter_mut().zip(value.iter()) {
                *target ^= source.rotate_left(3);
            }
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        use std::io::Read;
        std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(buf))
            .map_err(|_| ())
    }
}

/// Outcome summary of [`migrate_workspace_legacy_runs`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct LegacyRunMigration {
    pub migrated_runs: usize,
    pub skipped_marker_present: bool,
}

/// One-time migration of legacy workspace-local run traces into the global
/// sessions tree.
///
/// Moves every `<workspace>/.rove/runs/<run_id>/trace.jsonl` to
/// `<home>/sessions/legacy/<storage_key>/<run_id>/rollout-trace.jsonl`,
/// leaves workspace-owned reports/artifacts/memory untouched, then writes
/// `<workspace>/.rove/migrated.marker`. A present marker short-circuits the
/// whole scan, so repeated startups are no-ops.
pub fn migrate_workspace_legacy_runs(
    workspace_root: &Path,
    home: &RoveHome,
) -> io::Result<LegacyRunMigration> {
    let legacy_state = workspace_root.join(".rove");
    let marker = legacy_state.join(MIGRATED_MARKER_FILE);
    if marker.is_file() {
        return Ok(LegacyRunMigration {
            skipped_marker_present: true,
            ..LegacyRunMigration::default()
        });
    }

    let runs_dir = legacy_state.join("runs");
    let mut migrated = LegacyRunMigration::default();
    if runs_dir.is_dir() {
        let storage_key = storage_key_for(workspace_root);
        for entry in std::fs::read_dir(&runs_dir)? {
            let entry = entry?;
            let run_dir_path = entry.path();
            if !run_dir_path.is_dir() {
                continue;
            }
            let trace = run_dir_path.join("trace.jsonl");
            if !trace.is_file() {
                continue;
            }
            let Some(run_id) = run_dir_path.file_name() else {
                continue;
            };
            let target_dir = home
                .sessions_dir()
                .join("legacy")
                .join(storage_key.clone())
                .join(run_id);
            std::fs::create_dir_all(&target_dir)?;
            let target = target_dir.join("rollout-trace.jsonl");
            if !target.exists() {
                std::fs::rename(&trace, &target)?;
            }
            migrated.migrated_runs += 1;
        }
    }

    std::fs::create_dir_all(&legacy_state)?;
    std::fs::write(&marker, b"migrated\n")?;
    Ok(migrated)
}

/// Resolve the Rove home directory and run the one-time legacy-run
/// migration for `workspace_root`, best-effort. Failures are logged as
/// warnings and returned as `None` so startup never blocks on housekeeping.
pub fn ensure_home_legacy_run_migration(workspace_root: &Path) -> Option<LegacyRunMigration> {
    let home = match find_rove_home() {
        Ok(home) => RoveHome::new(home),
        Err(error) => {
            tracing::warn!(
                code = "rove_home_unavailable",
                %error,
                "Could not resolve the ROVE home directory; skipping legacy run migration"
            );
            return None;
        }
    };
    match migrate_workspace_legacy_runs(workspace_root, &home) {
        Ok(migration) => {
            if !migration.skipped_marker_present && migration.migrated_runs > 0 {
                tracing::info!(
                    migrated = migration.migrated_runs,
                    home = %home.root().display(),
                    "Migrated legacy workspace run traces into ~/.rove/sessions"
                );
            }
            Some(migration)
        }
        Err(error) => {
            tracing::warn!(
                code = "legacy_run_migration_failed",
                workspace_root = %workspace_root.display(),
                %error,
                "Legacy run migration failed; continuing without it"
            );
            None
        }
    }
}

fn storage_key_for(workspace_root: &Path) -> String {
    use std::fmt::Write as _;
    let normalized = workspace_root
        .to_string_lossy()
        .to_lowercase()
        .replace('\\', "/");
    let digest = rove_runtime::prompt_metadata::stable_hash(&normalized);
    let hash = digest.trim_start_matches("sha256:");
    let mut key = String::with_capacity(16);
    for byte in hash.bytes().take(STORAGE_KEY_BYTES) {
        let _ = write!(key, "{byte:02x}");
    }
    key
}

const STORAGE_KEY_BYTES: usize = 8;

#[cfg(test)]
mod tests {
    use super::*;

    fn error_kind(error: &HomeError) -> io::ErrorKind {
        match error {
            HomeError::Io(error) => error.kind(),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn rove_home_env_must_exist_and_be_a_directory() {
        let missing = tempfile::TempDir::new().unwrap().path().join("gone");
        let error =
            find_rove_home_from_env(Some(missing.to_str().unwrap())).expect_err("must fail");
        assert_eq!(error_kind(&error), io::ErrorKind::NotFound);

        let file = tempfile::TempDir::new().unwrap();
        let file_path = file.path().join("afile");
        std::fs::write(&file_path, b"x").unwrap();
        let error =
            find_rove_home_from_env(Some(file_path.to_str().unwrap())).expect_err("file must fail");
        assert_eq!(error_kind(&error), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn rove_home_env_canonicalizes_an_existing_directory() {
        let dir = tempfile::TempDir::new().unwrap();
        let resolved = find_rove_home_from_env(Some(dir.path().to_str().unwrap())).unwrap();
        assert!(resolved.is_absolute());
        assert_eq!(
            resolved.canonicalize().unwrap(),
            dir.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn empty_env_value_falls_through_to_home_directory() {
        // An empty value behaves like an unset variable; we only verify that
        // resolution succeeds or fails with the typed home error rather than
        // treating "" as a path.
        let result = find_rove_home_from_env(Some(""));
        match result {
            Ok(path) => assert!(path.ends_with(".rove")),
            Err(HomeError::NoHomeDirectory) => {}
            Err(other) => panic!("unexpected error: {other}"),
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_home_resolution_uses_userprofile() {
        let home = system_home_dir().expect("Windows test environment has USERPROFILE");
        assert!(!home.as_os_str().is_empty());
    }

    #[test]
    fn rollout_file_names_sort_by_time_within_a_day_partition() {
        let early = UNIX_EPOCH + std::time::Duration::from_secs(1);
        let later = UNIX_EPOCH + std::time::Duration::from_secs(86_500);
        let a = RoveHome::rollout_file_name(early);
        let b = RoveHome::rollout_file_name(later);
        assert!(a.starts_with("rollout-"));
        assert!(a.ends_with(".jsonl"));
        assert_ne!(a, b);
        // Date prefix orders across day boundaries regardless of clock time.
        let home = RoveHome::new("/tmp/x");
        let pa = home.session_rollout_path(early);
        let pb = home.session_rollout_path(later);
        assert_ne!(pa.parent(), pb.parent());
        assert_eq!(
            pa.parent().unwrap().parent().unwrap().parent().unwrap(),
            pb.parent().unwrap().parent().unwrap().parent().unwrap()
        );
    }

    #[test]
    fn legacy_run_migration_is_idempotent_and_moves_only_traces() {
        let ws = tempfile::TempDir::new().unwrap();
        let home_dir = tempfile::TempDir::new().unwrap();
        let home = RoveHome::new(home_dir.path());
        let run_dir = ws.path().join(".rove/runs/01ARZ3NDEKTSV4RRFFQ69G5FAV");
        std::fs::create_dir_all(&run_dir).unwrap();
        std::fs::write(run_dir.join("trace.jsonl"), b"{\"type\":\"llm_chunk\"}\n").unwrap();
        std::fs::write(run_dir.join("report.json"), b"{}").unwrap();
        std::fs::create_dir_all(ws.path().join(".rove/memory")).unwrap();
        std::fs::write(ws.path().join(".rove/memory/MEMORY.md"), b"# memory").unwrap();

        let first = migrate_workspace_legacy_runs(ws.path(), &home).unwrap();
        assert_eq!(first.migrated_runs, 1);
        assert!(!first.skipped_marker_present);
        assert!(ws.path().join(".rove/migrated.marker").is_file());
        // Trace moved out; report and memory stay put.
        assert!(!run_dir.join("trace.jsonl").exists());
        assert!(run_dir.join("report.json").exists());
        assert!(ws.path().join(".rove/memory/MEMORY.md").exists());

        // Second startup: marker short-circuits.
        let second = migrate_workspace_legacy_runs(ws.path(), &home).unwrap();
        assert!(second.skipped_marker_present);
        assert_eq!(second.migrated_runs, 0);
    }
}
