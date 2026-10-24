use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::game_dir::GameDir;

pub const CONFIG_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupsConfig {
    pub enabled: bool,
    pub keep: u32,
}

impl Default for BackupsConfig {
    fn default() -> Self {
        BackupsConfig {
            enabled: true,
            keep: 10,
        }
    }
}

/// The whole persisted state. This is the config file, one to one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub schema: u32,
    pub game_dirs: Vec<GameDir>,
    pub active_game_dir: Option<PathBuf>,
    pub last_world: Option<PathBuf>,
    pub theme: Theme,
    pub backups: BackupsConfig,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            schema: CONFIG_SCHEMA,
            game_dirs: Vec::new(),
            active_game_dir: None,
            last_world: None,
            theme: Theme::System,
            backups: BackupsConfig::default(),
        }
    }
}

impl Config {
    /// Add a validated dir. Returns false when the path is already known, so a
    /// repeated add converges to the same config.
    pub fn add_game_dir(&mut self, dir: GameDir) -> bool {
        if self.contains(&dir.path) {
            return false;
        }
        self.game_dirs.push(dir);
        true
    }

    /// Remove a stored dir by canonical path. Returns false when unknown.
    pub fn remove_game_dir(&mut self, path: &Path) -> bool {
        let before = self.game_dirs.len();
        self.game_dirs.retain(|dir| dir.path != path);
        if self.active_game_dir.as_deref() == Some(path) {
            self.active_game_dir = None;
        }
        self.game_dirs.len() != before
    }

    pub fn set_active_game_dir(&mut self, path: &Path) -> Result<(), AppError> {
        if !self.contains(path) {
            return Err(AppError::UnknownGameDir {
                path: path.to_path_buf(),
            });
        }
        self.active_game_dir = Some(path.to_path_buf());
        Ok(())
    }

    fn contains(&self, path: &Path) -> bool {
        self.game_dirs.iter().any(|dir| dir.path == path)
    }
}

/// Read the config file. A missing file is a fresh config, a bad file is an
/// error. The caller decides whether to recover.
pub fn load_config(path: &Path) -> Result<Config, AppError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(err) => return Err(err.into()),
    };
    let config: Config = serde_json::from_slice(&bytes).map_err(|err| AppError::Config {
        path: path.to_path_buf(),
        reason: format!("unreadable config: {err}"),
    })?;
    if config.schema != CONFIG_SCHEMA {
        return Err(AppError::Config {
            path: path.to_path_buf(),
            reason: format!(
                "unsupported schema {}, expected {CONFIG_SCHEMA}",
                config.schema
            ),
        });
    }
    Ok(config)
}

/// Write the config with a temp file, fsync, and rename. A crash leaves either
/// the old file or the new one, never a half file.
pub fn save_config(path: &Path, config: &Config) -> Result<(), AppError> {
    let dir = path.parent().ok_or_else(|| AppError::Config {
        path: path.to_path_buf(),
        reason: "config path has no parent directory".to_string(),
    })?;
    std::fs::create_dir_all(dir)?;
    let bytes = serde_json::to_vec_pretty(config).map_err(|err| AppError::Config {
        path: path.to_path_buf(),
        reason: format!("config does not serialize: {err}"),
    })?;

    let mut tmp = PathBuf::from(dir);
    tmp.push(format!(".config.json.{}.tmp", std::process::id()));
    let write_result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if let Err(err) = write_result {
        let _ = std::fs::remove_file(&tmp);
        return Err(err.into());
    }
    if let Ok(dir_handle) = std::fs::File::open(dir) {
        let _ = dir_handle.sync_all();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_dir::DiscoverySource;

    #[test]
    fn fresh_config_when_file_missing() {
        let root = tempfile::tempdir().unwrap();
        let config = load_config(&root.path().join("config.json")).unwrap();
        assert_eq!(config, Config::default());
        assert_eq!(config.schema, 1);
    }

    #[test]
    fn round_trips_through_disk() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("nested/config.json");
        let config = Config {
            game_dirs: vec![GameDir {
                path: PathBuf::from("/home/demo/.minecraft"),
                source: DiscoverySource::Manual,
            }],
            active_game_dir: Some(PathBuf::from("/home/demo/.minecraft")),
            theme: Theme::Dark,
            backups: BackupsConfig {
                enabled: false,
                keep: 3,
            },
            ..Config::default()
        };

        save_config(&path, &config).unwrap();
        let loaded = load_config(&path).unwrap();
        assert_eq!(loaded, config);
    }

    #[test]
    fn corrupt_config_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.json");
        std::fs::write(&path, b"{ not json").unwrap();

        let err = load_config(&path).unwrap_err();
        assert_eq!(err.code(), "config");
        assert!(
            err.to_string().contains(&path.display().to_string()),
            "error names the path: {err}"
        );
    }

    #[test]
    fn schema_mismatch_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.json");
        let config = Config {
            schema: 2,
            ..Config::default()
        };
        std::fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();

        let err = load_config(&path).unwrap_err();
        assert_eq!(err.code(), "config");
        assert!(err.to_string().contains("unsupported schema 2"), "{err}");
    }

    #[test]
    fn add_is_idempotent_and_remove_clears_active() {
        let mut config = Config::default();
        let dir = GameDir {
            path: PathBuf::from("/home/demo/.minecraft"),
            source: DiscoverySource::Manual,
        };
        assert!(config.add_game_dir(dir.clone()));
        assert!(!config.add_game_dir(dir.clone()));

        config.set_active_game_dir(&dir.path).unwrap();
        assert_eq!(config.active_game_dir.as_deref(), Some(dir.path.as_path()));

        assert!(config.remove_game_dir(&dir.path));
        assert!(!config.remove_game_dir(&dir.path));
        assert_eq!(config.active_game_dir, None);
    }

    #[test]
    fn active_dir_must_be_known() {
        let mut config = Config::default();
        let err = config
            .set_active_game_dir(Path::new("/nowhere"))
            .unwrap_err();
        assert_eq!(err.code(), "unknown_game_dir");
    }
}
