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

    /// Open an explicit absolute directory as a Folder workspace root.
    ///
    /// Unlike [`Self::detect`], this does **not** walk up to a parent git root:
    /// the provided path is the real execution boundary.
    pub fn open_folder(root: &Path) -> anyhow::Result<Self> {
        let root = open_existing_dir(root)?;
        Ok(Self {
            state_dir: root.join(".rove"),
            root,
            kind: WorkspaceKind::Folder,
        })
    }

    /// Open an explicit absolute directory as a Repo workspace root.
    ///
    /// Requires a `.git` entry **at** `root` (not only an ancestor). The provided
    /// path remains the real execution boundary.
    pub fn open_repo(root: &Path) -> anyhow::Result<Self> {
        let root = open_existing_dir(root)?;
        if !root.join(".git").exists() {
            anyhow::bail!(
                "repo workspace root must contain a .git entry: {}",
                root.display()
            );
        }
        Ok(Self {
            state_dir: root.join(".rove"),
            root,
            kind: WorkspaceKind::Repo,
        })
    }

    /// Ensure the `.rove/` state directory exists.
    pub fn ensure_state_dir(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.state_dir)
    }
}

fn open_existing_dir(root: &Path) -> anyhow::Result<PathBuf> {
    if !root.is_absolute() {
        anyhow::bail!(
            "workspace root must be an absolute path: {}",
            root.display()
        );
    }
    if !root.exists() {
        anyhow::bail!("workspace root does not exist: {}", root.display());
    }
    if !root.is_dir() {
        anyhow::bail!("workspace root must be a directory: {}", root.display());
    }
    root.canonicalize()
        .map_err(|err| anyhow::anyhow!("invalid workspace root {}: {err}", root.display()))
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

    #[test]
    fn open_folder_pins_explicit_root_even_when_git_exists() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join(".git")).unwrap();
        let nested = tmp.path().join("nested");
        std::fs::create_dir(&nested).unwrap();

        let ws = Workspace::open_folder(&nested).unwrap();

        assert_eq!(ws.kind, WorkspaceKind::Folder);
        assert_eq!(ws.root, nested.canonicalize().unwrap());
        assert_eq!(ws.state_dir, ws.root.join(".rove"));
    }

    #[test]
    fn open_repo_requires_git_at_root() {
        let tmp = TempDir::new().unwrap();
        let err = Workspace::open_repo(tmp.path()).unwrap_err();
        assert!(err.to_string().contains(".git"));

        std::fs::create_dir(tmp.path().join(".git")).unwrap();
        let ws = Workspace::open_repo(tmp.path()).unwrap();
        assert_eq!(ws.kind, WorkspaceKind::Repo);
        assert_eq!(ws.root, tmp.path().canonicalize().unwrap());
    }

    #[test]
    fn open_folder_rejects_relative_and_missing_paths() {
        let err = Workspace::open_folder(Path::new("relative/path")).unwrap_err();
        assert!(err.to_string().contains("absolute path"));

        let missing =
            std::env::temp_dir().join(format!("rove-missing-workspace-{}", ulid::Ulid::new()));
        let err = Workspace::open_folder(&missing).unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }

    #[test]
    fn open_folder_rejects_file_path() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("not-a-dir.txt");
        std::fs::write(&file, "x").unwrap();
        let err = Workspace::open_folder(&file).unwrap_err();
        assert!(err.to_string().contains("must be a directory"));
    }
}
