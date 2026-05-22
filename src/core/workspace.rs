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
}

/// Represents the world boundary the agent is working within.
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
}
