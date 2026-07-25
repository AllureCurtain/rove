use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The kind of workspace rove is operating in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceKind {
    /// A plain local directory.
    Folder,
    /// A directory with git repository semantics.
    Repo,
    /// An isolated standalone task workspace.
    Task,
}

/// Represents the runtime boundary the agent is working within.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    /// The root path of this workspace.
    pub root: PathBuf,
    /// What kind of workspace this is.
    pub kind: WorkspaceKind,
    /// Path to the `.rove/` state directory.
    pub state_dir: PathBuf,
}

impl Workspace {
    /// Detect workspace from a given directory.
    ///
    /// Walks up from `start_dir` looking for `.git` to determine if this is a Repo.
    /// Falls back to Folder if no git root is found.
    pub fn detect(start_dir: &Path) -> anyhow::Result<Self> {
        let start = start_dir
            .canonicalize()
            .unwrap_or_else(|_| start_dir.to_path_buf());

        let (root, kind) = if let Some(git_root) = find_git_root(&start) {
            (git_root, WorkspaceKind::Repo)
        } else {
            (start, WorkspaceKind::Folder)
        };

        let state_dir = root.join(".rove");

        Ok(Self {
            root,
            kind,
            state_dir,
        })
    }

    /// Create an isolated standalone task workspace under `base_dir`.
    pub fn task(base_dir: &Path, name: &str) -> anyhow::Result<Self> {
        let name = validate_task_workspace_name(name)?;
        let base = base_dir
            .canonicalize()
            .unwrap_or_else(|_| base_dir.to_path_buf());
        let root = base.join(name);
        let state_dir = root.join(".rove");
        std::fs::create_dir_all(&state_dir)?;

        Ok(Self {
            root,
            kind: WorkspaceKind::Task,
            state_dir,
        })
    }

    /// Ensure the `.rove/` state directory exists.
    pub fn ensure_state_dir(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.state_dir)
    }
}

/// Find the nearest ancestor (including self) that contains a `.git` directory.
fn find_git_root(path: &Path) -> Option<PathBuf> {
    let mut current = Some(path);
    while let Some(dir) = current {
        if dir.join(".git").exists() {
            return Some(dir.to_path_buf());
        }
        current = dir.parent();
    }
    None
}

fn validate_task_workspace_name(name: &str) -> anyhow::Result<&str> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || Path::new(trimmed).is_absolute()
        || Path::new(trimmed).components().count() != 1
        || trimmed == "."
        || trimmed == ".."
    {
        anyhow::bail!("invalid task workspace name: {name}");
    }
    Ok(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn detect_folder_workspace() {
        let tmp = TempDir::new().unwrap();
        let ws = Workspace::detect(tmp.path()).unwrap();
        assert_eq!(ws.kind, WorkspaceKind::Folder);
    }

    #[test]
    fn detect_repo_workspace() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join(".git")).unwrap();
        let ws = Workspace::detect(tmp.path()).unwrap();
        assert_eq!(ws.kind, WorkspaceKind::Repo);
    }

    #[test]
    fn detect_repo_workspace_from_subdirectory_uses_git_root() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join(".git")).unwrap();
        let nested = tmp.path().join("src").join("nested");
        std::fs::create_dir_all(&nested).unwrap();

        let ws = Workspace::detect(&nested).unwrap();

        assert_eq!(ws.kind, WorkspaceKind::Repo);
        assert_eq!(ws.root, tmp.path().canonicalize().unwrap());
        assert_eq!(
            ws.state_dir,
            tmp.path().canonicalize().unwrap().join(".rove")
        );
    }

    #[test]
    fn task_workspace_uses_isolated_root_and_state_dir_under_base() {
        let tmp = TempDir::new().unwrap();
        let ws = Workspace::task(tmp.path(), "standalone-task").unwrap();
        let expected_root = tmp.path().canonicalize().unwrap().join("standalone-task");

        assert_eq!(ws.kind, WorkspaceKind::Task);
        assert_eq!(ws.root, expected_root);
        assert_eq!(ws.state_dir, ws.root.join(".rove"));
        assert!(ws.root.exists());
        assert!(ws.state_dir.exists());
    }

    #[test]
    fn task_workspace_name_rejects_path_traversal() {
        let tmp = TempDir::new().unwrap();
        let err = Workspace::task(tmp.path(), "../escape").unwrap_err();

        assert!(err.to_string().contains("task workspace name"));
    }
}
