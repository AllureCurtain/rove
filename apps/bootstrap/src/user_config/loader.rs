use std::fs;

use super::{UserConfigDocument, UserConfigError, UserConfigPaths};

#[derive(Debug, Clone)]
pub struct UserConfigLoader {
    pub paths: UserConfigPaths,
}

impl UserConfigLoader {
    pub fn new(paths: UserConfigPaths) -> Self {
        Self { paths }
    }

    pub fn discover() -> Self {
        Self::new(UserConfigPaths::discover())
    }

    pub fn load(&self) -> Result<UserConfigDocument, UserConfigError> {
        self.paths.ensure_available()?;
        reject_symlink(&self.paths.root, "user provider configuration directory")?;
        reject_symlink(&self.paths.config_file, "user provider configuration")?;
        let text = match fs::read_to_string(&self.paths.config_file) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(UserConfigError::Missing {
                    path: self.paths.config_file.display().to_string(),
                });
            }
            Err(error) => {
                return Err(UserConfigError::Invalid {
                    message: format!("{}: {error}", self.paths.config_file.display()),
                });
            }
        };
        UserConfigDocument::from_toml(&text)
    }

    pub fn load_or_default(&self) -> Result<UserConfigDocument, UserConfigError> {
        match self.load() {
            Err(UserConfigError::Missing { .. }) => Ok(UserConfigDocument::default()),
            other => other,
        }
    }
}

fn reject_symlink(path: &std::path::Path, label: &str) -> Result<(), UserConfigError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(unix, windows))]
    #[test]
    fn loader_rejects_a_symlinked_catalog() {
        let temp = tempfile::TempDir::new().unwrap();
        let outside = temp.path().join("outside.toml");
        fs::write(&outside, "schema_version = 1").unwrap();
        let root = temp.path().join("user");
        fs::create_dir(&root).unwrap();
        let paths = UserConfigPaths::from_root(&root);
        if !create_file_symlink(&outside, &paths.config_file) {
            return;
        }
        let error = UserConfigLoader::new(paths).load().unwrap_err();
        assert!(error.to_string().contains("symbolic link"));
    }

    #[cfg(unix)]
    fn create_file_symlink(target: &std::path::Path, link: &std::path::Path) -> bool {
        std::os::unix::fs::symlink(target, link).is_ok()
    }

    #[cfg(windows)]
    fn create_file_symlink(target: &std::path::Path, link: &std::path::Path) -> bool {
        std::os::windows::fs::symlink_file(target, link).is_ok()
    }
}
