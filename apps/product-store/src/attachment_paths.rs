//! Path rules for session attachment payloads.
//!
//! The layout is `<data_root>/attachments/<product_session_id>/<attachment_id>`.
//! These helpers hold the containment rules both the store (metadata-only
//! availability) and the API byte routes depend on, so the two cannot disagree
//! about which paths are safe.

use std::path::{Component, Path, PathBuf};

use crate::ProductAttachmentRecord;

/// The directory name under the data root. Its presence is part of the
/// `<data_root>` contract in `docs/architecture.md`.
pub const ATTACHMENTS_DIR: &str = "attachments";

/// Resolve the attachment root from the path that already owns the product
/// database: `<product.sqlite parent>/attachments`.
///
/// The root therefore follows the pinned user-data root, and a relative
/// database path yields a relative attachment root in the same directory the
/// database itself would use.
pub fn attachments_root(product_sqlite_path: &Path) -> PathBuf {
    product_sqlite_path
        .parent()
        .map(|parent| parent.join(ATTACHMENTS_DIR))
        .unwrap_or_else(|| PathBuf::from(ATTACHMENTS_DIR))
}

/// The payload path for one durable row, for a caller that needs to look at the
/// file's metadata and not at its bytes.
///
/// Deliberately synchronous and allocation-cheap: the transcript projection runs
/// once per message and must not hash or open a payload. `None` means the row
/// names something that is not a single safe path component, which the caller
/// reports as corruption rather than following.
pub fn payload_path_for(root: &Path, record: &ProductAttachmentRecord) -> Option<PathBuf> {
    let directory = session_dir_in(root, record.product_session_id.as_str()).ok()?;
    payload_path_in(&directory, record.attachment_id.as_str()).ok()
}

/// Join one server-generated id under a canonical root through the same
/// `join_safe` discipline the workspace file surface uses, then require that
/// the result is still inside the root.
pub fn session_dir_in(root: &Path, session_id: &str) -> Result<PathBuf, String> {
    require_single_component(session_id, "attachment session id")?;
    let directory = join_safe(root, session_id)?;
    if !directory.starts_with(root) {
        return Err("attachment session directory escapes the attachment root".to_string());
    }
    Ok(directory)
}

/// The payload path for a server-generated id.
///
/// `join_safe` refuses an absolute path, a `..` component, and a secret-shaped
/// component, and re-checks a canonicalised path against the root — so a link
/// planted at the payload path cannot redirect a read outside the session
/// directory. The result is deliberately **not** canonicalised further: the
/// read path must be able to see that the final component is a symbolic link.
pub fn payload_path_in(session_dir: &Path, attachment_id: &str) -> Result<PathBuf, String> {
    require_single_component(attachment_id, "attachment id")?;
    join_safe(session_dir, attachment_id)
}

/// A single plain path component, so a crafted id can never add a directory
/// level or escape the session directory even before `join_safe` runs.
///
/// `Path` alone is not the last word on identity where the payloads live: see
/// [`is_portable_component`]. Ids are validated ULIDs in every production path,
/// so both checks are defence in depth for the join itself.
fn require_single_component(value: &str, label: &'static str) -> Result<(), String> {
    let mut components = Path::new(value).components();
    let single = matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(_)), None)
    );
    if single && is_portable_component(value) {
        return Ok(());
    }
    Err(format!("{label} is not a single safe path component"))
}

/// A component a Windows path cannot reinterpret.
///
/// The containment rules on that platform are not the ones [`Path`] reports:
/// a trailing dot or space is stripped, so `payload.` and `payload` are one
/// file; `:` introduces an alternate data stream; and a reserved device name
/// (`CON`, `NUL`, `COM1`, …) names a device even as a directory entry, with or
/// without an extension. Refusing them here is what gives this checkout's own
/// platform a *non-skipping* negative case: the symbolic-link defences cannot be
/// created without a privilege on Windows, but these names are refused on every
/// platform by the same helper — including the Linux CI runner, which is why
/// this rule is stated over the *value* and not over `Path`'s parse of it.
///
/// A bare UNC or `\\.\` prefix is deliberately not in this rule. On Windows
/// `Path` reports it as a prefix rather than one `Normal` component, so
/// [`require_single_component`] refuses it there; on Unix those bytes are one
/// ordinary file name that `join_safe` keeps inside the session directory, so
/// refusing it here would change behaviour rather than extend the same
/// guarantee. `a_component_this_platform_would_reinterpret_is_refused` states
/// that split case by case.
fn is_portable_component(value: &str) -> bool {
    if value.is_empty() || value.ends_with('.') || value.ends_with(' ') || value.contains(':') {
        return false;
    }
    let stem = value.split('.').next().unwrap_or(value);
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    !RESERVED.iter().any(|name| stem.eq_ignore_ascii_case(name))
}

/// Join a workspace- or root-relative path without leaving `root`.
///
/// Refuses an absolute path, a `..` component, and a secret-shaped component,
/// and re-checks a canonicalised result against `root` so a planted link cannot
/// redirect outside it. The workspace file surface and the attachment store
/// share this one implementation.
pub fn join_safe(root: &Path, relative: &str) -> Result<PathBuf, String> {
    if relative.is_empty() {
        return Ok(root.to_path_buf());
    }
    let rel = Path::new(relative);
    if rel.is_absolute() {
        return Err("path must be workspace-relative".to_string());
    }
    let mut out = root.to_path_buf();
    for component in rel.components() {
        match component {
            Component::Normal(part) => {
                let name = part.to_string_lossy();
                if is_secret_filename(&name) {
                    return Err("hidden or secret-shaped path".to_string());
                }
                out.push(part);
            }
            Component::CurDir => {}
            Component::ParentDir => {
                return Err("path traversal blocked".to_string());
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err("absolute paths are not allowed".to_string());
            }
        }
    }
    if let Ok(canonical) = out.canonicalize() {
        if !canonical.starts_with(root) {
            return Err("symlink escapes workspace".to_string());
        }
        Ok(canonical)
    } else {
        Ok(out)
    }
}

pub fn is_secret_filename(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with(".env")
        || lower.ends_with(".pem")
        || lower.ends_with(".key")
        || lower.starts_with("id_rsa")
        || lower.starts_with("id_ed25519")
        || lower == ".npmrc"
        || lower == ".netrc"
        || lower == ".dockercfg"
        || lower == "credentials.json"
        || lower.ends_with(".p12")
        || lower.ends_with(".pfx")
}
