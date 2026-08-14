use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use fs2::FileExt;

use super::{UserConfigDocument, UserConfigError, UserConfigLoader, UserConfigPaths};

const LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const LOCK_RETRY: Duration = Duration::from_millis(10);
static PROCESS_WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone)]
pub struct UserConfigWriter {
    pub paths: UserConfigPaths,
}

impl UserConfigWriter {
    pub fn new(paths: UserConfigPaths) -> Self {
        Self { paths }
    }

    pub fn update(
        &self,
        expected_revision: Option<&str>,
        document: &UserConfigDocument,
    ) -> Result<UserConfigDocument, UserConfigError> {
        document.validate()?;
        let _process_guard = PROCESS_WRITE_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        fs::create_dir_all(&self.paths.root).map_err(|error| UserConfigError::Invalid {
            message: format!("could not create config directory: {error}"),
        })?;
        reject_symlink(&self.paths.root, "user provider configuration directory")?;
        reject_symlink(&self.paths.config_file, "user provider configuration")?;
        reject_symlink(&self.paths.lock_file, "user provider configuration lock")?;
        restrict_directory_permissions(&self.paths.root)?;
        let _lock = ConfigFileLock::acquire(&self.paths.lock_file)?;
        restrict_file_permissions(&self.paths.lock_file)?;
        let current = UserConfigLoader::new(self.paths.clone()).load_or_default()?;
        if expected_revision.is_some_and(|expected| expected != current.revision()) {
            return Err(UserConfigError::RevisionConflict);
        }
        let text = document.to_toml()?;
        let mut temp = tempfile::NamedTempFile::new_in(&self.paths.root).map_err(|error| {
            UserConfigError::Invalid {
                message: format!("could not create temporary configuration: {error}"),
            }
        })?;
        temp.write_all(text.as_bytes())
            .and_then(|_| temp.as_file().sync_all())
            .map_err(|error| UserConfigError::Invalid {
                message: format!("could not flush temporary configuration: {error}"),
            })?;
        temp.persist(&self.paths.config_file)
            .map_err(|error| UserConfigError::Invalid {
                message: format!(
                    "could not atomically replace configuration: {}",
                    error.error
                ),
            })?;
        restrict_file_permissions(&self.paths.config_file)?;
        sync_parent(&self.paths.root)?;
        Ok(document.clone())
    }
}

fn reject_symlink(path: &Path, label: &str) -> Result<(), UserConfigError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(UserConfigError::Invalid {
            message: format!("{label} must not be a symbolic link"),
        }),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(UserConfigError::Invalid {
            message: format!("could not inspect {label}: {error}"),
        }),
    }
}

#[cfg(unix)]
fn restrict_directory_permissions(path: &Path) -> Result<(), UserConfigError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
        UserConfigError::Invalid {
            message: format!("could not restrict config directory permissions: {error}"),
        }
    })
}

#[cfg(not(unix))]
fn restrict_directory_permissions(_path: &Path) -> Result<(), UserConfigError> {
    Ok(())
}

#[cfg(unix)]
fn restrict_file_permissions(path: &Path) -> Result<(), UserConfigError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|error| {
        UserConfigError::Invalid {
            message: format!("could not restrict config file permissions: {error}"),
        }
    })
}

#[cfg(not(unix))]
fn restrict_file_permissions(_path: &Path) -> Result<(), UserConfigError> {
    Ok(())
}

struct ConfigFileLock {
    file: File,
}

impl ConfigFileLock {
    fn acquire(path: &Path) -> Result<Self, UserConfigError> {
        let deadline = Instant::now() + LOCK_TIMEOUT;
        loop {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(path)
                .map_err(|error| UserConfigError::Invalid {
                    message: format!("could not open provider configuration lock: {error}"),
                })?;
            match file.try_lock_exclusive() {
                Ok(()) => return Ok(Self { file }),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(UserConfigError::Busy);
                    }
                    thread::sleep(LOCK_RETRY);
                }
                Err(error) => {
                    return Err(UserConfigError::Invalid {
                        message: format!("could not acquire provider configuration lock: {error}"),
                    });
                }
            }
        }
    }
}

impl Drop for ConfigFileLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> Result<(), UserConfigError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| UserConfigError::Invalid {
            message: format!("could not flush provider configuration directory: {error}"),
        })
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> Result<(), UserConfigError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use super::*;

    #[test]
    fn writer_uses_revision_cas_and_atomic_replace() {
        let temp = tempfile::TempDir::new().unwrap();
        let paths = UserConfigPaths::from_root(temp.path());
        let writer = UserConfigWriter::new(paths.clone());
        let mut document = UserConfigDocument::default();
        writer.update(None, &document).unwrap();
        let revision = UserConfigLoader::new(paths.clone())
            .load()
            .unwrap()
            .revision();
        document.model.default_model = Some("changed".to_string());
        writer.update(Some(&revision), &document).unwrap();
        let conflict = writer.update(Some("sha256:stale"), &document).unwrap_err();
        assert!(conflict.to_string().contains("revision conflict"));
        assert!(paths.lock_file.exists());
    }

    #[test]
    fn concurrent_updates_cannot_lose_a_write() {
        let temp = tempfile::TempDir::new().unwrap();
        let paths = UserConfigPaths::from_root(temp.path());
        let writer = UserConfigWriter::new(paths.clone());
        let initial = writer.update(None, &UserConfigDocument::default()).unwrap();
        let expected = initial.revision();
        let barrier = Arc::new(Barrier::new(3));
        let mut threads = Vec::new();
        for model in ["first", "second"] {
            let barrier = Arc::clone(&barrier);
            let paths = paths.clone();
            let expected = expected.clone();
            threads.push(std::thread::spawn(move || {
                let mut document = UserConfigDocument::default();
                document.model.default_model = Some(model.to_string());
                barrier.wait();
                UserConfigWriter::new(paths).update(Some(&expected), &document)
            }));
        }
        barrier.wait();
        let results = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            results.iter().filter(|result| result.is_ok()).count(),
            1,
            "results: {results:?}"
        );
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(UserConfigError::RevisionConflict)))
                .count(),
            1
        );
    }
}
