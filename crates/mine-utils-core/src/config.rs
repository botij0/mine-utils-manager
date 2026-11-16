use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::game_dir::{GameDir, GameDirEntry};

pub const CONFIG_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "wire-types", derive(ts_rs::TS))]
pub enum Theme {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(feature = "wire-types", derive(ts_rs::TS))]
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
#[serde(default)]
#[cfg_attr(feature = "wire-types", derive(ts_rs::TS))]
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
        if let Some(last_world) = self.last_world.as_ref() {
            if last_world.as_path() == path || last_world.starts_with(path) {
                self.last_world = None;
            }
        }
        self.game_dirs.len() != before
    }

    /// Mark a dir active. The caller validates that the path is known, since
    /// detection spans more than the dirs stored here.
    pub fn set_active_game_dir(&mut self, path: &Path) {
        if self.active_game_dir.as_deref() != Some(path) {
            self.last_world = None;
        }
        self.active_game_dir = Some(path.to_path_buf());
    }

    fn contains(&self, path: &Path) -> bool {
        self.game_dirs.iter().any(|dir| dir.path == path)
    }
}

/// Owns the config file and the live config. Every mutation writes the new
/// config to disk before it becomes the in-memory value, so a failed write
/// cannot leave memory ahead of disk.
pub struct ConfigStore {
    path: PathBuf,
    config: Config,
}

impl ConfigStore {
    pub fn new(path: PathBuf, config: Config) -> Self {
        ConfigStore { path, config }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn add_game_dir(&mut self, dir: GameDir) -> Result<bool, AppError> {
        let mut next = self.config.clone();
        if !next.add_game_dir(dir) {
            return Ok(false);
        }
        self.commit(next)?;
        Ok(true)
    }

    pub fn remove_game_dir(&mut self, path: &Path) -> Result<(), AppError> {
        let mut next = self.config.clone();
        if !next.remove_game_dir(path) {
            return Err(AppError::UnknownGameDir {
                path: path.to_path_buf(),
            });
        }
        self.commit(next)
    }

    pub fn set_active_game_dir(
        &mut self,
        path: &Path,
        entries: &[GameDirEntry],
        discovered: bool,
    ) -> Result<(), AppError> {
        let entry = entries
            .iter()
            .find(|entry| entry.path == path)
            .ok_or_else(|| AppError::UnknownGameDir {
                path: path.to_path_buf(),
            })?;
        if !entry.present {
            return Err(AppError::InvalidGameDir {
                path: path.to_path_buf(),
                reason: "expected one of versions, saves, or resourcepacks".to_string(),
            });
        }
        if !discovered && !self.config.game_dirs.iter().any(|dir| dir.path == path) {
            return Err(AppError::UnknownGameDir {
                path: path.to_path_buf(),
            });
        }
        let mut next = self.config.clone();
        next.set_active_game_dir(path);
        self.commit(next)
    }

    pub fn set_last_world(&mut self, world: &str) -> Result<(), AppError> {
        let game = self
            .config
            .active_game_dir
            .as_deref()
            .ok_or(AppError::NoActiveGameDir)?;
        let world = crate::world::WorldPaths::for_world(game, world)?;
        let path = world.world_dir().to_path_buf();
        if !crate::world::is_level_file(&world.level_dat_path()) {
            return Err(AppError::World {
                reason: format!("{} is not a world in this game dir", path.display()),
            });
        }
        let mut next = self.config.clone();
        next.last_world = Some(path);
        self.commit(next)
    }

    pub fn set_theme(&mut self, theme: Theme) -> Result<(), AppError> {
        let mut next = self.config.clone();
        next.theme = theme;
        self.commit(next)
    }

    fn commit(&mut self, next: Config) -> Result<(), AppError> {
        save_config(&self.path, &next)?;
        self.config = next;
        Ok(())
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

        config.set_active_game_dir(&dir.path);
        assert_eq!(config.active_game_dir.as_deref(), Some(dir.path.as_path()));

        assert!(config.remove_game_dir(&dir.path));
        assert!(!config.remove_game_dir(&dir.path));
        assert_eq!(config.active_game_dir, None);
    }

    #[test]
    fn set_theme_keeps_game_dirs() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.json");
        let game = root.path().join("game");
        let mut config = Config::default();
        config.add_game_dir(GameDir {
            path: game.clone(),
            source: DiscoverySource::Manual,
        });
        let mut store = ConfigStore::new(path.clone(), config);

        store.set_theme(Theme::Dark).unwrap();

        let loaded = load_config(&path).unwrap();
        assert_eq!(loaded.theme, Theme::Dark);
        assert_eq!(loaded.game_dirs.len(), 1);
        assert_eq!(loaded.game_dirs[0].path, game);
    }

    #[test]
    fn failed_persist_keeps_memory() {
        let root = tempfile::tempdir().unwrap();
        let blocker = root.path().join("blocker");
        std::fs::write(&blocker, b"x").unwrap();
        let path = blocker.join("config.json");
        let mut store = ConfigStore::new(path, Config::default());

        assert!(store.set_theme(Theme::Dark).is_err());
        assert_eq!(store.config().theme, Theme::System);
        assert!(store.config().game_dirs.is_empty());
    }

    #[test]
    fn remove_missing_stored_dir() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.json");
        let missing = root.path().join("missing");
        let mut config = Config::default();
        config.add_game_dir(GameDir {
            path: missing.clone(),
            source: DiscoverySource::Manual,
        });
        let mut store = ConfigStore::new(path, config);

        store.remove_game_dir(&missing).unwrap();

        assert!(store.config().game_dirs.is_empty());
    }

    #[test]
    fn remove_active_clears_active_and_last_world() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.json");
        let game = root.path().join("game");
        let world = game.join("saves").join("world");
        let mut config = Config::default();
        config.add_game_dir(GameDir {
            path: game.clone(),
            source: DiscoverySource::Manual,
        });
        config.set_active_game_dir(&game);
        config.last_world = Some(world);
        let mut store = ConfigStore::new(path, config);

        store.remove_game_dir(&game).unwrap();

        assert_eq!(store.config().active_game_dir, None);
        assert_eq!(store.config().last_world, None);
    }

    #[test]
    fn set_active_rejects_missing_entry() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.json");
        let game = root.path().join("game");
        let entries = vec![GameDirEntry {
            path: game.clone(),
            source: DiscoverySource::Manual,
            present: false,
            stored: true,
        }];
        let mut store = ConfigStore::new(path, Config::default());

        let err = store
            .set_active_game_dir(&game, &entries, false)
            .unwrap_err();

        assert_eq!(err.code(), "invalid_game_dir");
        assert_eq!(store.config().active_game_dir, None);
    }

    #[test]
    fn removed_manual_directory_cannot_be_selected_from_stale_listing() {
        let root = tempfile::tempdir().unwrap();
        let game = root.path().join("game");
        let mut config = Config::default();
        config.add_game_dir(GameDir {
            path: game.clone(),
            source: DiscoverySource::Manual,
        });
        let entries = vec![GameDirEntry {
            path: game.clone(),
            source: DiscoverySource::Manual,
            present: true,
            stored: true,
        }];
        let mut store = ConfigStore::new(root.path().join("config.json"), config);
        store.remove_game_dir(&game).unwrap();
        assert_eq!(
            store
                .set_active_game_dir(&game, &entries, false)
                .unwrap_err()
                .code(),
            "unknown_game_dir"
        );
        assert_eq!(store.config().active_game_dir, None);
    }

    #[test]
    fn last_world_is_persisted_and_cleared_when_game_changes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.json");
        let game = root.path().join("game");
        let world = game.join("saves/creative");
        std::fs::create_dir_all(&world).unwrap();
        std::fs::write(world.join("level.dat"), b"level").unwrap();
        let mut config = Config::default();
        config.set_active_game_dir(&game);
        let mut store = ConfigStore::new(path.clone(), config);
        store.set_last_world("creative").unwrap();
        assert_eq!(load_config(&path).unwrap().last_world, Some(world));
        store.config.set_active_game_dir(&root.path().join("other"));
        assert_eq!(store.config().last_world, None);
    }

    #[test]
    fn last_world_refuses_a_saves_folder_without_level_dat() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.json");
        let game = root.path().join("game");
        std::fs::create_dir_all(game.join("saves/plain")).unwrap();
        let mut config = Config::default();
        config.set_active_game_dir(&game);
        let mut store = ConfigStore::new(path.clone(), config);
        let err = store.set_last_world("plain").unwrap_err();
        assert_eq!(err.code(), "world");
        assert_eq!(store.config().last_world, None);
        assert!(!path.exists());
    }

    #[test]
    fn missing_fields_use_defaults() {
        let config: Config = serde_json::from_str(r#"{"schema":1,"theme":"dark"}"#).unwrap();

        assert_eq!(config.theme, Theme::Dark);
        assert!(config.game_dirs.is_empty());
        assert_eq!(config.active_game_dir, None);
        assert_eq!(config.last_world, None);
        assert_eq!(config.backups, BackupsConfig::default());
    }
}
