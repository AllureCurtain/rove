use std::path::{Path, PathBuf};

pub const USER_CONFIG_ROOT_ENV: &str = "ROVE_CONFIG_ROOT";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserConfigPaths {
    pub root: PathBuf,
    pub config_file: PathBuf,
    pub lock_file: PathBuf,
}

impl UserConfigPaths {
    pub fn discover() -> Self {
        if let Some(root) = std::env::var_os(USER_CONFIG_ROOT_ENV).map(PathBuf::from) {
            return Self::from_root(root);
        }
        let root = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".rove");
        Self::from_root(root)
    }

    pub fn from_root(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            config_file: root.join("config.toml"),
            lock_file: root.join("config.toml.lock"),
            root,
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
        }
    }
}
