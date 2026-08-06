use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const TRUSTED_WORKSPACES_ENV: &str = "ROVE_TRUSTED_WORKSPACES";

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectActivationState {
    #[default]
    Restricted,
    Trusted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectActivationSource {
    Programmatic,
    CommandLine,
    Environment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectActivation {
    pub state: ProjectActivationState,
    pub source: Option<ProjectActivationSource>,
    pub trusted_workspace_roots: Vec<PathBuf>,
}

impl ProjectActivation {
    pub(crate) fn resolve(
        workspace_root: &Path,
        command_line_grant: bool,
        trusted_workspaces: Option<OsString>,
    ) -> anyhow::Result<Self> {
        let workspace_root = canonical_directory(workspace_root)?;
        let mut trusted_workspace_roots = Vec::new();
        let mut seen = HashSet::new();

        if let Some(raw) = trusted_workspaces {
            for path in std::env::split_paths(&raw) {
                if path.as_os_str().is_empty() {
                    continue;
                }
                let path = canonical_directory(&path).map_err(|error| {
                    anyhow::anyhow!(
                        "{TRUSTED_WORKSPACES_ENV} contains an invalid workspace path: {error}"
                    )
                })?;
                if seen.insert(path.clone()) {
                    trusted_workspace_roots.push(path);
                }
            }
        }

        if command_line_grant && seen.insert(workspace_root.clone()) {
            trusted_workspace_roots.push(workspace_root.clone());
        }

        let trusted = trusted_workspace_roots.contains(&workspace_root);
        let source = if command_line_grant {
            Some(ProjectActivationSource::CommandLine)
        } else if trusted {
            Some(ProjectActivationSource::Environment)
        } else {
            None
        };
        Ok(Self {
            state: if trusted {
                ProjectActivationState::Trusted
            } else {
                ProjectActivationState::Restricted
            },
            source,
            trusted_workspace_roots,
        })
    }

    pub(crate) fn programmatic() -> Self {
        Self {
            state: ProjectActivationState::Trusted,
            source: Some(ProjectActivationSource::Programmatic),
            trusted_workspace_roots: Vec::new(),
        }
    }

    pub(crate) fn for_workspace(&self, workspace_root: &Path) -> Self {
        if self.source == Some(ProjectActivationSource::Programmatic) {
            return Self::programmatic();
        }
        let trusted = self
            .trusted_workspace_roots
            .contains(&workspace_root.to_path_buf());
        Self {
            state: if trusted {
                ProjectActivationState::Trusted
            } else {
                ProjectActivationState::Restricted
            },
            source: if trusted {
                self.source.or(Some(ProjectActivationSource::Environment))
            } else {
                None
            },
            trusted_workspace_roots: self.trusted_workspace_roots.clone(),
        }
    }
}

fn canonical_directory(path: &Path) -> anyhow::Result<PathBuf> {
    let canonical = path
        .canonicalize()
        .map_err(|error| anyhow::anyhow!("{}: {error}", path.display()))?;
    if !canonical.is_dir() {
        anyhow::bail!("{} is not a directory", canonical.display());
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::{ProjectActivation, ProjectActivationSource, ProjectActivationState};

    #[test]
    fn command_line_grant_is_bound_to_the_selected_workspace() {
        let selected = tempfile::TempDir::new().unwrap();
        let other = tempfile::TempDir::new().unwrap();
        let activation = ProjectActivation::resolve(selected.path(), true, None).unwrap();

        assert_eq!(activation.state, ProjectActivationState::Trusted);
        assert_eq!(
            activation.source,
            Some(ProjectActivationSource::CommandLine)
        );
        assert_eq!(
            activation.for_workspace(other.path()).state,
            ProjectActivationState::Restricted
        );
    }
}
