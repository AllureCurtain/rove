use std::fs::TryLockError;
use std::fs::{File, OpenOptions};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum ApiOwnershipError {
    #[error(
        "Another Rove instance is already using this data directory. Close it before starting another instance."
    )]
    Busy,
    #[error("Rove could not lock its data directory; check that it is a writable local directory.")]
    Unavailable(#[source] std::io::Error),
}

/// The file stays in place: unlinking it would let a peer lock another inode.
#[derive(Debug)]
pub(crate) struct ApiOwnership {
    _file: File,
}

impl ApiOwnership {
    pub(crate) fn acquire(database: &Path) -> Result<Self, ApiOwnershipError> {
        Self::open(database)
            .map_err(ApiOwnershipError::Unavailable)
            .and_then(|file| match file.try_lock() {
                Ok(()) => Ok(Self { _file: file }),
                Err(TryLockError::WouldBlock) => Err(ApiOwnershipError::Busy),
                Err(TryLockError::Error(error)) => Err(ApiOwnershipError::Unavailable(error)),
            })
    }

    fn open(database: &Path) -> std::io::Result<File> {
        let parent = database
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or_else(|| std::io::Error::other("data directory is missing"))?;
        std::fs::create_dir_all(parent)?;
        let parent = parent.canonicalize()?;
        let path = parent.join(".rove-api.lock");
        if let Ok(metadata) = path.symlink_metadata()
            && (!metadata.is_file() || metadata.file_type().is_symlink())
        {
            return Err(std::io::Error::other(
                "ownership lock is not a regular file",
            ));
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
        }
        let file = options.open(&path)?;
        if !file.metadata()?.is_file() || file.metadata()?.file_type().is_symlink() {
            return Err(std::io::Error::other(
                "ownership lock is not a regular file",
            ));
        }
        Ok(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    #[test]
    fn cancelled_database_waiters_retain_ownership_until_blocking_workers_finish() {
        let root = tempfile::tempdir().unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(1)
            .build()
            .unwrap();
        runtime.block_on(async {
            let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            let blocker = tokio::task::spawn_blocking(move || {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            });
            entered_rx.await.unwrap();
            let database = root.path().join("product.sqlite");
            let guard = std::sync::Arc::new(ApiOwnership::acquire(&database).unwrap());
            let store = crate::product::store::open_product_store_with_guard(
                database.clone(),
                5_000,
                guard.clone(),
            )
            .unwrap();
            let index = crate::StateIndex::new(root.path()).with_lifetime_guard(guard.clone());
            index.initialize().unwrap();
            {
                let read = store.list_workspaces();
                tokio::pin!(read);
                assert!(futures::poll!(&mut read).is_pending());
                let write = index.mark_running_jobs_interrupted_async();
                tokio::pin!(write);
                assert!(futures::poll!(&mut write).is_pending());
            }
            drop(store);
            drop(index);
            drop(guard);
            let retained = matches!(
                ApiOwnership::acquire(&database),
                Err(ApiOwnershipError::Busy)
            );
            release_tx.send(()).unwrap();
            blocker.await.unwrap();
            assert!(
                retained,
                "dropping async waiters must not release queued database workers' guard"
            );
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    match ApiOwnership::acquire(&database) {
                        Ok(_) => break,
                        Err(ApiOwnershipError::Busy) => tokio::task::yield_now().await,
                        Err(error) => panic!("{error}"),
                    }
                }
            })
            .await
            .unwrap();
        });
    }

    #[test]
    fn ownership_contends_across_processes_and_releases_on_process_death() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("product.sqlite");
        let ready = root.path().join("ready");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "host_ownership::tests::ownership_child",
                "--nocapture",
            ])
            .env("ROVE_OWNERSHIP_TEST_ROOT", root.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !ready.exists() {
            if child.try_wait().unwrap().is_some() || std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("child failed to acquire ownership");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let denied = matches!(
            ApiOwnership::acquire(&database),
            Err(ApiOwnershipError::Busy)
        );
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(denied);
        let acquired = ApiOwnership::acquire(&database).unwrap();
        assert!(root.path().join(".rove-api.lock").is_file());
        assert!(matches!(
            ApiOwnership::acquire(&database),
            Err(ApiOwnershipError::Busy)
        ));
        let other = root.path().join("other/product.sqlite");
        let _other = ApiOwnership::acquire(&other).unwrap();
        drop(acquired);
        assert!(ApiOwnership::acquire(&database).is_ok());
    }

    #[test]
    fn ownership_child() {
        let Some(root) = std::env::var_os("ROVE_OWNERSHIP_TEST_ROOT") else {
            return;
        };
        let root = std::path::PathBuf::from(root);
        let _lock = ApiOwnership::acquire(&root.join("product.sqlite")).unwrap();
        std::fs::write(root.join("ready"), b"ready").unwrap();
        std::thread::sleep(std::time::Duration::from_secs(30));
    }

    #[cfg(unix)]
    #[test]
    fn aliases_contend_and_symlink_or_special_lock_files_are_refused() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let _lock = ApiOwnership::acquire(&data.join("product.sqlite")).unwrap();
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&data, &alias).unwrap();
        assert!(matches!(
            ApiOwnership::acquire(&alias.join("product.sqlite")),
            Err(ApiOwnershipError::Busy)
        ));
        let bad = root.path().join("bad");
        std::fs::create_dir(&bad).unwrap();
        let target = root.path().join("must-not-change");
        std::fs::write(&target, "preserved").unwrap();
        std::os::unix::fs::symlink(&target, bad.join(".rove-api.lock")).unwrap();
        assert!(matches!(
            ApiOwnership::acquire(&bad.join("product.sqlite")),
            Err(ApiOwnershipError::Unavailable(_))
        ));
        assert_eq!(std::fs::read_to_string(target).unwrap(), "preserved");
        std::fs::remove_file(bad.join(".rove-api.lock")).unwrap();
        std::fs::create_dir(bad.join(".rove-api.lock")).unwrap();
        assert!(matches!(
            ApiOwnership::acquire(&bad.join("product.sqlite")),
            Err(ApiOwnershipError::Unavailable(_))
        ));
    }
}
