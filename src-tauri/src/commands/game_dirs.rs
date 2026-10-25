use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, Once};

use mine_utils_core::{
    discover_game_dirs, entry_for, home_dir, merge_game_dirs, validate_game_dir, AppError,
    BackupsConfig, Config, ConfigStore, DataPaths, GameDirEntry, StackPaths, Theme, WorldPaths,
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

    /// The backup policy from the config. Config writes stay backend-owned.
    pub fn backups_config(&self) -> BackupsConfig {
        self.lock().config().backups.clone()
    }

    /// Where stack writes go: the active game dir's `options.txt`, its backup
    /// folder under the app data dir, and the backup policy.
    pub fn stack_paths(&self) -> Result<StackPaths, AppError> {
        let game_dir = self.active_game_dir().ok_or(AppError::NoActiveGameDir)?;
        Ok(StackPaths::for_game_dir(
            &game_dir,
            &self.data_dir,
            self.backups_config(),
        ))
    }

    /// Where one world's datapack stack lives: its `level.dat`, its
    /// `datapacks` dir, and its backup folder under the app data dir.
    pub fn world_data_paths(&self, world: &str) -> Result<DataPaths, AppError> {
        let game_dir = self.active_game_dir().ok_or(AppError::NoActiveGameDir)?;
        let resolved = WorldPaths::for_world(&game_dir, world)?;
        Ok(DataPaths::for_world(
            &game_dir,
            &resolved.world_dir,
            &self.data_dir,
            self.backups_config(),
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
