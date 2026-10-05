use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::ModelSelection;

const LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const LOCK_RETRY: Duration = Duration::from_millis(10);
const MAX_SELECTION_BYTES: u64 = 64 * 1024;
static PROCESS_SELECTION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PersistedSessionSelection {
    pub schema_version: u16,
    pub revision: u64,
    pub selection: ModelSelection,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionSelectionError {
    #[error("session model selection revision conflict")]
    RevisionConflict,
    #[error("session model selection is busy")]
    Busy,
    #[error("session model selection is invalid or unavailable: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone)]
pub struct SessionSelectionStore {
    root: PathBuf,
}

impl SessionSelectionStore {
    pub fn new(state_root: impl Into<PathBuf>) -> Self {
        Self {
            root: state_root.into().join("session-model-selections"),
        }
    }

    pub fn load(
        &self,
        session_id: &str,
    ) -> Result<Option<PersistedSessionSelection>, SessionSelectionError> {
        let path = self.selection_path(session_id)?;
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(invalid(error)),
        };
        if bytes.len() as u64 > MAX_SELECTION_BYTES {
            return Err(SessionSelectionError::Invalid(
                "selection file exceeds the size limit".to_string(),
            ));
        }
        let value: PersistedSessionSelection = serde_json::from_slice(&bytes).map_err(|_| {
            SessionSelectionError::Invalid("selection file is malformed".to_string())
        })?;
        if value.schema_version != 1 || value.revision == 0 {
            return Err(SessionSelectionError::Invalid(
                "selection schema or revision is unsupported".to_string(),
            ));
        }
        Ok(Some(value))
    }

    pub fn update(
        &self,
        session_id: &str,
        expected_revision: u64,
        selection: ModelSelection,
    ) -> Result<PersistedSessionSelection, SessionSelectionError> {
        let path = self.selection_path(session_id)?;
        let _process_guard = PROCESS_SELECTION_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        fs::create_dir_all(&self.root).map_err(invalid)?;
        reject_symlink(&self.root)?;
        restrict_directory_permissions(&self.root)?;
        let lock_path = path.with_extension("json.lock");
        reject_symlink(&path)?;
        reject_symlink(&lock_path)?;
        let _lock = SelectionFileLock::acquire(&lock_path)?;
        restrict_file_permissions(&lock_path)?;
        let current = self.load(session_id)?;
        let current_revision = current.as_ref().map_or(0, |value| value.revision);
        if expected_revision != current_revision {
            return Err(SessionSelectionError::RevisionConflict);
        }
        let revision = current_revision.checked_add(1).ok_or_else(|| {
            SessionSelectionError::Invalid("selection revision overflow".to_string())
        })?;
        let value = PersistedSessionSelection {
            schema_version: 1,
            revision,
            selection,
        };
        let encoded = serde_json::to_vec_pretty(&value).map_err(|_| {
            SessionSelectionError::Invalid("selection serialization failed".to_string())
        })?;
        let mut temp = tempfile::NamedTempFile::new_in(&self.root).map_err(invalid)?;
        temp.write_all(&encoded)
            .and_then(|_| temp.as_file().sync_all())
            .map_err(invalid)?;
        temp.persist(&path).map_err(|error| invalid(error.error))?;
        restrict_file_permissions(&path)?;
        sync_parent(&self.root)?;
        Ok(value)
    }

    fn selection_path(&self, session_id: &str) -> Result<PathBuf, SessionSelectionError> {
        if session_id.is_empty()
            || session_id.len() > 128
            || !session_id.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            return Err(SessionSelectionError::Invalid(
                "session id is invalid".to_string(),
            ));
        }
        Ok(self.root.join(format!("{session_id}.json")))
    }
}

fn reject_symlink(path: &Path) -> Result<(), SessionSelectionError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(SessionSelectionError::Invalid(
            "session selection path must not be a symbolic link".to_string(),
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(invalid(error)),
    }
}

#[cfg(unix)]
fn restrict_directory_permissions(path: &Path) -> Result<(), SessionSelectionError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(invalid)
}

#[cfg(not(unix))]
fn restrict_directory_permissions(_: &Path) -> Result<(), SessionSelectionError> {
    Ok(())
}

#[cfg(unix)]
fn restrict_file_permissions(path: &Path) -> Result<(), SessionSelectionError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(invalid)
}

#[cfg(not(unix))]
fn restrict_file_permissions(_: &Path) -> Result<(), SessionSelectionError> {
    Ok(())
}

struct SelectionFileLock(File);

impl SelectionFileLock {
    fn acquire(path: &Path) -> Result<Self, SessionSelectionError> {
        let deadline = Instant::now() + LOCK_TIMEOUT;
        loop {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(path)
                .map_err(invalid)?;
            match file.try_lock_exclusive() {
                Ok(()) => return Ok(Self(file)),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(SessionSelectionError::Busy);
                    }
                    thread::sleep(LOCK_RETRY);
                }
                Err(error) => return Err(invalid(error)),
            }
        }
    }
}

impl Drop for SelectionFileLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

fn invalid(error: impl std::fmt::Display) -> SessionSelectionError {
    SessionSelectionError::Invalid(error.to_string())
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> Result<(), SessionSelectionError> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(invalid)
}

#[cfg(not(unix))]
fn sync_parent(_: &Path) -> Result<(), SessionSelectionError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProviderProfileId;

    fn selection(revision: &str, model: &str) -> ModelSelection {
        ModelSelection {
            profile_id: ProviderProfileId::new("local").unwrap(),
            model: model.to_string(),
            reasoning: "default".to_string(),
            revision: revision.to_string(),
        }
    }

    #[test]
    fn session_selection_is_atomic_cas_and_secret_free() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = SessionSelectionStore::new(temp.path());
        let first = store
            .update("01JSESSION", 0, selection("catalog-a", "m1"))
            .unwrap();
        assert_eq!(first.revision, 1);
        let conflict = store
            .update("01JSESSION", 0, selection("catalog-b", "m2"))
            .unwrap_err();
        assert!(matches!(conflict, SessionSelectionError::RevisionConflict));
        let loaded = store.load("01JSESSION").unwrap().unwrap();
        assert_eq!(loaded, first);
        let encoded = serde_json::to_string(&loaded).unwrap();
        assert!(!encoded.contains("credential"));
        assert!(!encoded.contains("endpoint"));
    }
}
