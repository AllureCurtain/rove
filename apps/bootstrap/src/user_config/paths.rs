use std::path::{Path, PathBuf};

pub const USER_CONFIG_ROOT_ENV: &str = "ROVE_CONFIG_ROOT";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserConfigPaths {
    pub root: PathBuf,
    pub config_file: PathBuf,
    pub lock_file: PathBuf,
    discovery_error: Option<String>,
}

impl UserConfigPaths {
    pub fn discover() -> Self {
        Self::discover_from(
            std::env::var_os(USER_CONFIG_ROOT_ENV).map(PathBuf::from),
            std::env::var_os("USERPROFILE").map(PathBuf::from),
            std::env::var_os("HOME").map(PathBuf::from),
        )
    }

    pub fn from_root(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            config_file: root.join("config.toml"),
            lock_file: root.join("config.toml.lock"),
            root,
            discovery_error: None,
        }
    }

    pub fn for_config_file(path: impl AsRef<Path>) -> Self {
        let config_file = path.as_ref().to_path_buf();
        let root = config_file
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            lock_file: config_file.with_extension("toml.lock"),
            config_file,
            root,
            discovery_error: None,
        }
    }

    pub(super) fn ensure_available(&self) -> Result<(), super::UserConfigError> {
        match &self.discovery_error {
            Some(message) => Err(super::UserConfigError::Unavailable {
                message: message.clone(),
            }),
            None => Ok(()),
        }
    }

    fn discover_from(
        configured_root: Option<PathBuf>,
        user_profile: Option<PathBuf>,
        home: Option<PathBuf>,
    ) -> Self {
        if let Some(root) = configured_root {
            return if root.is_absolute() {
                Self::from_root(root)
            } else {
                Self::unavailable(format!("{USER_CONFIG_ROOT_ENV} must be an absolute path"))
            };
        }
        match user_profile.or(home) {
            Some(root) if root.is_absolute() => Self::from_root(root.join(".rove")),
            Some(_) => Self::unavailable(
                "the user home directory must resolve to an absolute path".to_string(),
            ),
            None => Self::unavailable(
                "the user configuration root is unavailable because USERPROFILE and HOME are not set"
                    .to_string(),
            ),
        }
    }

    fn unavailable(message: String) -> Self {
        Self {
            root: PathBuf::new(),
            config_file: PathBuf::new(),
            lock_file: PathBuf::new(),
            discovery_error: Some(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_fails_closed_without_an_absolute_configuration_root() {
        let missing = UserConfigPaths::discover_from(None, None, None);
        assert!(missing.ensure_available().is_err());
        assert!(missing.config_file.as_os_str().is_empty());

        let relative =
            UserConfigPaths::discover_from(Some(PathBuf::from("relative-config")), None, None);
        assert!(relative.ensure_available().is_err());
    }
}
