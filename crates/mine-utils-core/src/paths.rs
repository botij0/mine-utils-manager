use std::path::PathBuf;

use directories::{ProjectDirs, UserDirs};

use crate::error::AppError;

/// XDG locations for the app. The shell resolves these once and passes them in.
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl AppPaths {
    pub fn resolve() -> Result<AppPaths, AppError> {
        let dirs =
            ProjectDirs::from("dev", "botij0", "mine-utils").ok_or_else(|| AppError::Config {
                path: PathBuf::from("~"),
                reason: "cannot resolve the home directory".to_string(),
            })?;
        Ok(AppPaths {
            config_dir: dirs.config_dir().to_path_buf(),
            data_dir: dirs.data_dir().to_path_buf(),
            cache_dir: dirs.cache_dir().to_path_buf(),
        })
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.json")
    }
}

/// The user home dir, the root of game dir discovery.
pub fn home_dir() -> Result<PathBuf, AppError> {
    UserDirs::new()
        .map(|dirs| dirs.home_dir().to_path_buf())
        .ok_or_else(|| AppError::Config {
            path: PathBuf::from("~"),
            reason: "cannot resolve the home directory".to_string(),
        })
}
