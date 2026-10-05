//! Cross-process serialization for SQLite schema migrations.
//!
//! rove has two entry points that may start at the same moment: the desktop
//! API process and a short-lived CLI/TUI invocation. Both open the same state
//! database and both run pending migrations on the way in. SQLite alone does
//! not make that safe: a migration sequence is several statements, and the
//! decision to run one is a read followed by a write. Two processes can both
//! read "not applied" and both execute the DDL.
//!
//! This module provides the outer barrier. The rule for callers is:
//!
//! 1. Read the applied schema version on a normal connection. If it is already
//!    current, return without taking the lock — the hot path stays lock-free.
//! 2. Otherwise take this lock, re-read the version inside it (double-checked
//!    locking), and only then apply migrations.
//!
//! The lock is advisory and file-based rather than a SQLite transaction so the
//! whole multi-statement sequence is covered, and so a backfill task can wait
//! on the same barrier without holding a write transaction open.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fs2::FileExt;

/// How long a caller waits for a peer's migration to finish before giving up.
///
/// Sized well above a realistic migration so contention resolves by waiting,
/// and below any human-visible startup budget so a stuck peer surfaces as a
/// diagnosable error instead of a hang.
pub const MIGRATION_LOCK_TIMEOUT: Duration = Duration::from_secs(30);
const MIGRATION_LOCK_RETRY: Duration = Duration::from_millis(25);
const MIGRATION_LOCK_SUFFIX: &str = ".migrate.lock";

/// Why a migration barrier could not be taken.
///
/// Contention that outlives the timeout is reported separately from an IO
/// failure so callers can say "another process is migrating" rather than
/// collapsing both into a generic unavailable-store error.
#[derive(Debug)]
pub enum MigrationLockError {
    /// A peer held the barrier for longer than [`MIGRATION_LOCK_TIMEOUT`].
    Timeout { path: PathBuf, waited: Duration },
    /// The lock file itself could not be opened or locked.
    Io { path: PathBuf, reason: String },
}

impl std::fmt::Display for MigrationLockError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout { path, waited } => write!(
                formatter,
                "timed out after {:?} waiting for another process to finish migrating {}",
                waited,
                path.display()
            ),
            Self::Io { path, reason } => write!(
                formatter,
                "could not acquire migration lock {}: {reason}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for MigrationLockError {}

impl From<MigrationLockError> for std::io::Error {
    fn from(error: MigrationLockError) -> Self {
        let kind = match &error {
            MigrationLockError::Timeout { .. } => std::io::ErrorKind::TimedOut,
            MigrationLockError::Io { .. } => std::io::ErrorKind::Other,
        };
        std::io::Error::new(kind, error.to_string())
    }
}

/// An held migration barrier. Released on drop, including on panic and on
/// process exit — the OS drops advisory locks with the file handle, so a killed
/// migrator cannot wedge the barrier permanently.
#[derive(Debug)]
pub struct MigrationLock {
    file: File,
    path: PathBuf,
}

impl MigrationLock {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for MigrationLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

/// The lock path for a database: a sibling file, never a fixed global path.
///
/// Deriving it from the database keeps per-workspace and per-test databases
/// independent, so one test's migration cannot serialize against another's.
pub fn migration_lock_path(database_path: &Path) -> PathBuf {
    let mut name = database_path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(MIGRATION_LOCK_SUFFIX);
    match database_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(name),
        _ => PathBuf::from(name),
    }
}

/// Take the migration barrier for `database_path`, waiting out contention.
pub fn acquire_migration_lock(database_path: &Path) -> Result<MigrationLock, MigrationLockError> {
    acquire_migration_lock_with_timeout(database_path, MIGRATION_LOCK_TIMEOUT)
}

/// Timeout-parameterized form, so tests can assert the contention path without
/// waiting the production budget.
pub fn acquire_migration_lock_with_timeout(
    database_path: &Path,
    timeout: Duration,
) -> Result<MigrationLock, MigrationLockError> {
    let path = migration_lock_path(database_path);
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|error| MigrationLockError::Io {
            path: path.clone(),
            reason: error.to_string(),
        })?;
    }
    // A lock file that is not a regular file (directory, symlink, device) is
    // refused rather than followed: the barrier must not become a way to reach
    // an unexpected path.
    if let Ok(metadata) = std::fs::symlink_metadata(&path)
        && (!metadata.is_file() || metadata.file_type().is_symlink())
    {
        return Err(MigrationLockError::Io {
            path,
            reason: "migration lock path is not a regular file".to_string(),
        });
    }
    let file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|error| MigrationLockError::Io {
            path: path.clone(),
            reason: error.to_string(),
        })?;

    let started = Instant::now();
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(MigrationLock { file, path }),
            Err(error) if is_lock_contention(&error) => {
                let waited = started.elapsed();
                if waited >= timeout {
                    return Err(MigrationLockError::Timeout { path, waited });
                }
                std::thread::sleep(MIGRATION_LOCK_RETRY.min(timeout.saturating_sub(waited)));
            }
            Err(error) => {
                return Err(MigrationLockError::Io {
                    path,
                    reason: error.to_string(),
                });
            }
        }
    }
}

/// Distinguish "someone else holds it" from a real IO failure.
///
/// Windows reports contention as `ERROR_SHARING_VIOLATION` (32) or
/// `ERROR_LOCK_VIOLATION` (33) rather than `WouldBlock`.
fn is_lock_contention(error: &std::io::Error) -> bool {
    if error.kind() == std::io::ErrorKind::WouldBlock {
        return true;
    }
    #[cfg(windows)]
    {
        matches!(error.raw_os_error(), Some(32 | 33))
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lock_lives_beside_its_database_not_at_a_shared_global_path() {
        let first = migration_lock_path(Path::new("/tmp/a/state.sqlite"));
        let second = migration_lock_path(Path::new("/tmp/b/state.sqlite"));
        assert_ne!(first, second);
        assert_eq!(first.file_name().unwrap(), "state.sqlite.migrate.lock");
        assert_eq!(first.parent().unwrap(), Path::new("/tmp/a"));
    }

    #[test]
    fn a_second_acquisition_waits_and_then_reports_contention_not_success() {
        let temp = tempfile::TempDir::new().unwrap();
        let database = temp.path().join("state.sqlite");
        let held = acquire_migration_lock(&database).unwrap();

        let error = acquire_migration_lock_with_timeout(&database, Duration::from_millis(120))
            .expect_err("a held barrier must not be handed out twice");
        match error {
            MigrationLockError::Timeout { waited, .. } => {
                assert!(
                    waited >= Duration::from_millis(100),
                    "must actually wait out the timeout, waited {waited:?}"
                );
            }
            other => panic!("expected a timeout, got {other:?}"),
        }

        drop(held);
        acquire_migration_lock(&database).expect("barrier must be reusable once released");
    }

    #[test]
    fn a_timeout_maps_to_a_timed_out_io_error_rather_than_a_generic_failure() {
        let temp = tempfile::TempDir::new().unwrap();
        let database = temp.path().join("state.sqlite");
        let _held = acquire_migration_lock(&database).unwrap();
        let error = acquire_migration_lock_with_timeout(&database, Duration::from_millis(60))
            .expect_err("contended");
        let io: std::io::Error = error.into();
        assert_eq!(io.kind(), std::io::ErrorKind::TimedOut);
    }

    #[test]
    fn a_lock_path_that_is_a_directory_is_refused_rather_than_used() {
        let temp = tempfile::TempDir::new().unwrap();
        let database = temp.path().join("state.sqlite");
        std::fs::create_dir_all(migration_lock_path(&database)).unwrap();
        let error = acquire_migration_lock(&database).expect_err("a directory is not a lock file");
        assert!(matches!(error, MigrationLockError::Io { .. }));
    }

    #[test]
    fn releasing_the_barrier_is_observable_to_a_waiting_peer() {
        let temp = tempfile::TempDir::new().unwrap();
        let database = temp.path().join("state.sqlite");
        let held = acquire_migration_lock(&database).unwrap();
        let path = database.clone();
        let waiter = std::thread::spawn(move || {
            acquire_migration_lock_with_timeout(&path, Duration::from_secs(10))
                .map(|lock| lock.path().to_path_buf())
        });
        std::thread::sleep(Duration::from_millis(80));
        drop(held);
        let acquired = waiter.join().unwrap().expect("waiter should acquire");
        assert_eq!(acquired, migration_lock_path(&database));
    }
}
