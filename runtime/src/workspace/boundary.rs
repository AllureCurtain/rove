use std::path::{Component, Path, PathBuf};

use crate::types::ApprovalPolicy;
use rove_core::{ToolDescriptor, ToolError};

/// Check whether a tool call is allowed under the active approval policy.
pub fn check_tool_allowed(
    schema: &ToolDescriptor,
    policy: ApprovalPolicy,
) -> Result<(), ToolError> {
    match (schema.destructive, policy) {
        (true, ApprovalPolicy::Never) => Err(ToolError::PermissionDenied {
            reason: "destructive tool blocked by policy".to_string(),
        }),
        (true, ApprovalPolicy::Ask) => Err(ToolError::PermissionDenied {
            reason: "destructive tool requires explicit approval".to_string(),
        }),
        _ => Ok(()),
    }
}

/// Resolve a workspace-relative path for reading.
///
/// The final target must already exist and its canonical path must remain under
/// the canonical workspace root, which rejects symlink/reparse-point escapes.
pub fn resolve_workspace_read_path(root: &Path, raw_path: &str) -> Result<PathBuf, ToolError> {
    let canonical_root = canonical_workspace_root(root)?;
    let relative = normalize_relative_path(raw_path)?;
    let candidate = canonical_root.join(relative);
    let canonical_target = candidate
        .canonicalize()
        .map_err(|err| ToolError::InvalidInput {
            reason: format!("invalid path: {err}"),
        })?;
    ensure_under_workspace(&canonical_root, &canonical_target)?;
    Ok(canonical_target)
}

/// Resolve an existing workspace-relative path while refusing every linked
/// component, even when the link target would remain inside the workspace.
///
/// Authority-bearing package and procedure discovery uses this stricter form:
/// provenance must describe the actual tracked path rather than a linked alias.
pub fn resolve_workspace_read_path_without_links(
    root: &Path,
    raw_path: &str,
) -> Result<PathBuf, ToolError> {
    let canonical_root = canonical_workspace_root(root)?;
    let relative = normalize_relative_path(raw_path)?;
    reject_symlinked_components(&canonical_root, &relative)?;
    let candidate = canonical_root.join(relative);
    let canonical_target = candidate
        .canonicalize()
        .map_err(|err| ToolError::InvalidInput {
            reason: format!("invalid path: {err}"),
        })?;
    ensure_under_workspace(&canonical_root, &canonical_target)?;
    Ok(canonical_target)
}

/// Resolve a workspace-relative path for writing.
///
/// Existing targets are canonicalized directly. New targets canonicalize their
/// nearest existing ancestor, so normal new files are allowed while writes
/// through symlinked ancestors or existing symlink files are rejected.
pub fn resolve_workspace_write_path(root: &Path, raw_path: &str) -> Result<PathBuf, ToolError> {
    let canonical_root = canonical_workspace_root(root)?;
    let relative = normalize_relative_path(raw_path)?;
    let candidate = canonical_root.join(&relative);
    reject_symlinked_components(&canonical_root, &relative)?;

    if candidate.exists() {
        let canonical_target = candidate
            .canonicalize()
            .map_err(|err| ToolError::InvalidInput {
                reason: format!("invalid path: {err}"),
            })?;
        ensure_under_workspace(&canonical_root, &canonical_target)?;
        return Ok(canonical_target);
    }

    let mut ancestor = candidate.parent().unwrap_or(canonical_root.as_path());
    while !ancestor.exists() {
        ancestor = ancestor.parent().ok_or_else(|| ToolError::InvalidInput {
            reason: "path has no existing workspace ancestor".to_string(),
        })?;
    }
    let canonical_ancestor = ancestor
        .canonicalize()
        .map_err(|err| ToolError::InvalidInput {
            reason: format!("invalid path ancestor: {err}"),
        })?;
    ensure_under_workspace(&canonical_root, &canonical_ancestor)?;
    Ok(candidate)
}

fn reject_symlinked_components(root: &Path, relative: &Path) -> Result<(), ToolError> {
    let mut candidate = root.to_path_buf();
    for component in relative.components() {
        candidate.push(component.as_os_str());
        match std::fs::symlink_metadata(&candidate) {
            Ok(metadata) if is_symlink_or_reparse(&metadata) => {
                return Err(ToolError::PermissionDenied {
                    reason: "writes through symlink/reparse paths are not allowed".to_string(),
                });
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(ToolError::InvalidInput {
                    reason: format!("invalid path metadata: {error}"),
                });
            }
        }
    }
    Ok(())
}

/// Whether a path entry is a symlink or a Windows reparse point.
///
/// Crate-visible so the Agent package loader applies the same rule rooted at a
/// package directory. Duplicating the reparse-point attribute check would leave
/// two places to keep correct.
pub fn is_symlink_or_reparse(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn canonical_workspace_root(root: &Path) -> Result<PathBuf, ToolError> {
    root.canonicalize().map_err(|err| ToolError::InvalidInput {
        reason: format!("invalid workspace root: {err}"),
    })
}

fn normalize_relative_path(raw_path: &str) -> Result<PathBuf, ToolError> {
    let path = PathBuf::from(raw_path);
    if path.is_absolute() {
        return Err(ToolError::InvalidInput {
            reason: "absolute paths are not allowed".to_string(),
        });
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(value) => normalized.push(value),
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(ToolError::PermissionDenied {
                        reason: "path escapes workspace".to_string(),
                    });
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(ToolError::InvalidInput {
                    reason: "absolute paths are not allowed".to_string(),
                });
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err(ToolError::InvalidInput {
            reason: "path must not be empty".to_string(),
        });
    }

    Ok(normalized)
}

fn ensure_under_workspace(root: &Path, target: &Path) -> Result<(), ToolError> {
    if target.starts_with(root) {
        Ok(())
    } else {
        Err(ToolError::PermissionDenied {
            reason: "path escapes workspace".to_string(),
        })
    }
}
