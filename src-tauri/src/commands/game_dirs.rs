use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, Once};

use mine_utils_core::{
    discover_game_dirs, entry_for, home_dir, level_backup, merge_game_dirs, validate_game_dir,
    AppError, BackupsConfig, Config, ConfigStore, DataPaths, GameDirEntry, StackPaths, Theme,
    WorldPaths,
};
use serde::Serialize;
use tauri::State;

pub struct AppState {
    store: Mutex<ConfigStore>,
    startup_warning: Option<String>,
    first_ipc: Once,
    data_dir: PathBuf,
}

impl AppState {
    pub fn new(store: ConfigStore, startup_warning: Option<String>, data_dir: PathBuf) -> Self {
        AppState {
            store: Mutex::new(store),
            startup_warning,
            first_ipc: Once::new(),
            data_dir,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ConfigStore> {
        self.store.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn note_first_ipc(&self) {
        self.first_ipc.call_once(|| log::info!("first-ipc"));
    }

    pub fn note_ipc(&self) {
        self.note_first_ipc();
    }

    /// The selected game dir, if one is configured. Pack commands resolve
    /// `resourcepacks` from it.
    pub fn active_game_dir(&self) -> Option<PathBuf> {
        self.lock().config().active_game_dir.clone()
    }

    /// One consistent read of the active game dir and its backup policy.
    /// Every write command derives its paths from this snapshot, so a config
    /// change mid-command cannot split paths across two game dirs.
    pub fn snapshot(&self) -> Result<GameDirSnapshot, AppError> {
        let (game_dir, backups) = {
            let store = self.lock();
            let config = store.config();
            let game_dir = config
                .active_game_dir
                .clone()
                .ok_or(AppError::NoActiveGameDir)?;
            (game_dir, config.backups.clone())
        };
        Ok(GameDirSnapshot {
            game_dir,
            data_dir: self.data_dir.clone(),
            backups,
        })
    }

    /// Where stack writes go: the active game dir's `options.txt`, its backup
    /// folder under the app data dir, and the backup policy.
    pub fn stack_paths(&self) -> Result<StackPaths, AppError> {
        Ok(self.snapshot()?.stack_paths())
    }

    /// Where one world's datapack stack lives: its `level.dat`, its
    /// `datapacks` dir, and its backup folder under the app data dir.
    pub fn world_data_paths(&self, world: &str) -> Result<DataPaths, AppError> {
        self.snapshot()?.world_data_paths(world)
    }

    /// The shared backup folder for the active game dir's `level.dat` files.
    pub fn level_backups_root(&self) -> Result<PathBuf, AppError> {
        let game_dir = self.active_game_dir().ok_or(AppError::NoActiveGameDir)?;
        Ok(level_backup::root_for(&self.data_dir, &game_dir))
    }
}

#[derive(Debug, Clone)]
pub struct GameDirSnapshot {
    pub game_dir: PathBuf,
    pub data_dir: PathBuf,
    backups: BackupsConfig,
}

impl GameDirSnapshot {
    pub fn stack_paths(&self) -> StackPaths {
        StackPaths::for_game_dir(&self.game_dir, &self.data_dir, self.backups.clone())
    }

    /// The datapack write context for a target world, derived from the
    /// snapshot without re-reading the config.
    pub fn world_data_paths(&self, world: &str) -> Result<DataPaths, AppError> {
        let resolved = WorldPaths::for_world(&self.game_dir, world)?;
        Ok(DataPaths::for_world(
            &self.game_dir,
            &resolved.world_dir,
            &self.data_dir,
            self.backups.clone(),
        ))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GameDirState {
    pub config: Config,
    pub entries: Vec<GameDirEntry>,
}

fn listing(config: &Config) -> Result<Vec<GameDirEntry>, AppError> {
    let home = home_dir()?;
    let merged = merge_game_dirs(discover_game_dirs(&home), &config.game_dirs);
    let stored: HashSet<PathBuf> = config
        .game_dirs
        .iter()
        .map(|dir| dir.path.clone())
        .collect();
    Ok(merged
        .into_iter()
        .map(|dir| {
            let mut entry = entry_for(&dir);
            entry.stored = stored.contains(&dir.path);
            entry
        })
        .collect())
}

fn state_for(store: &ConfigStore) -> Result<GameDirState, AppError> {
    let config = store.config().clone();
    let entries = listing(&config)?;
    Ok(GameDirState { config, entries })
}

#[tauri::command]
pub fn list_game_dirs(state: State<'_, AppState>) -> Result<Vec<GameDirEntry>, AppError> {
    state.note_first_ipc();
    let store = state.lock();
    listing(store.config())
}

#[tauri::command]
pub fn add_game_dir(state: State<'_, AppState>, path: String) -> Result<GameDirState, AppError> {
    state.note_first_ipc();
    let dir = validate_game_dir(Path::new(&path))?;
    let mut store = state.lock();
    store.add_game_dir(dir)?;
    state_for(&store)
}

#[tauri::command]
pub fn remove_game_dir(state: State<'_, AppState>, path: String) -> Result<GameDirState, AppError> {
    state.note_first_ipc();
    let target = std::fs::canonicalize(Path::new(&path)).unwrap_or_else(|_| PathBuf::from(&path));
    let mut store = state.lock();
    store.remove_game_dir(&target)?;
    state_for(&store)
}

#[tauri::command]
pub fn set_active_game_dir(
    state: State<'_, AppState>,
    path: String,
) -> Result<GameDirState, AppError> {
    state.note_first_ipc();
    let target = std::fs::canonicalize(Path::new(&path)).map_err(|_| AppError::UnknownGameDir {
        path: PathBuf::from(&path),
    })?;
    let mut store = state.lock();
    let entries = listing(store.config())?;
    store.set_active_game_dir(&target, &entries)?;
    state_for(&store)
}

#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> Result<Config, AppError> {
    state.note_first_ipc();
    Ok(state.lock().config().clone())
}

#[tauri::command]
pub fn set_theme(state: State<'_, AppState>, theme: Theme) -> Result<Config, AppError> {
    state.note_first_ipc();
    let mut store = state.lock();
    store.set_theme(theme)?;
    Ok(store.config().clone())
}

#[tauri::command]
pub fn get_startup_warning(state: State<'_, AppState>) -> Result<Option<String>, AppError> {
    state.note_first_ipc();
    Ok(state.startup_warning.clone())
}
