use std::path::{Component, Path, PathBuf};

use crate::types::ApprovalPolicy;
use rove_core::{ToolDescriptor as ToolSchema, ToolError};

/// Check whether a tool call is allowed under the active approval policy.
pub fn check_tool_allowed(schema: &ToolSchema, policy: ApprovalPolicy) -> Result<(), ToolError> {
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

/// Resolve a workspace-relative path for writing.
///
/// Existing targets are canonicalized directly. New targets canonicalize their
/// nearest existing ancestor, so normal new files are allowed while writes
/// through symlinked ancestors or existing symlink files are rejected.
pub fn resolve_workspace_write_path(root: &Path, raw_path: &str) -> Result<PathBuf, ToolError> {
    let canonical_root = canonical_workspace_root(root)?;
    let relative = normalize_relative_path(raw_path)?;
    let candidate = canonical_root.join(&relative);

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
