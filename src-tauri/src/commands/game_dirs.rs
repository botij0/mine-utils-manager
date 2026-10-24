use std::path::{Path, PathBuf};
use std::sync::{Mutex, Once};

use mine_utils_core::{
    contains_entry, discover_game_dirs, entry_for, home_dir, merge_game_dirs,
    save_config as persist_config, validate_game_dir, AppError, AppPaths, Config, GameDirEntry,
    CONFIG_SCHEMA,
};
use tauri::State;

pub struct AppState {
    pub paths: AppPaths,
    config: Mutex<Config>,
    first_ipc: Once,
}

impl AppState {
    pub fn new(paths: AppPaths, config: Config) -> Self {
        AppState {
            paths,
            config: Mutex::new(config),
            first_ipc: Once::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Config> {
        self.config.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn note_first_ipc(&self) {
        self.first_ipc.call_once(|| log::info!("first-ipc"));
    }
}

fn listing(config: &Config) -> Result<Vec<GameDirEntry>, AppError> {
    let home = home_dir()?;
    let merged = merge_game_dirs(discover_game_dirs(&home), &config.game_dirs);
    Ok(merged.iter().map(entry_for).collect())
}

fn canonical(path: &Path) -> Result<PathBuf, AppError> {
    std::fs::canonicalize(path).map_err(|_| AppError::UnknownGameDir {
        path: path.to_path_buf(),
    })
}

#[tauri::command]
pub fn list_game_dirs(state: State<'_, AppState>) -> Result<Vec<GameDirEntry>, AppError> {
    state.note_first_ipc();
    let config = state.lock();
    listing(&config)
}

#[tauri::command]
pub fn add_game_dir(
    state: State<'_, AppState>,
    path: String,
) -> Result<Vec<GameDirEntry>, AppError> {
    state.note_first_ipc();
    let dir = validate_game_dir(Path::new(&path))?;
    let mut config = state.lock();
    config.add_game_dir(dir);
    persist_config(&state.paths.config_file(), &config)?;
    listing(&config)
}

#[tauri::command]
pub fn remove_game_dir(
    state: State<'_, AppState>,
    path: String,
) -> Result<Vec<GameDirEntry>, AppError> {
    state.note_first_ipc();
    let target = canonical(Path::new(&path))?;
    let mut config = state.lock();
    if !config.remove_game_dir(&target) {
        return Err(AppError::UnknownGameDir { path: target });
    }
    persist_config(&state.paths.config_file(), &config)?;
    listing(&config)
}

#[tauri::command]
pub fn set_active_game_dir(state: State<'_, AppState>, path: String) -> Result<Config, AppError> {
    state.note_first_ipc();
    let target = canonical(Path::new(&path))?;
    let mut config = state.lock();
    let known = listing(&config)?;
    if !contains_entry(&known, &target) {
        return Err(AppError::UnknownGameDir { path: target });
    }
    config.set_active_game_dir(&target);
    persist_config(&state.paths.config_file(), &config)?;
    Ok(config.clone())
}

#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> Result<Config, AppError> {
    state.note_first_ipc();
    Ok(state.lock().clone())
}

#[tauri::command]
pub fn save_config(state: State<'_, AppState>, config: Config) -> Result<Config, AppError> {
    state.note_first_ipc();
    if config.schema != CONFIG_SCHEMA {
        return Err(AppError::Config {
            path: state.paths.config_file(),
            reason: format!(
                "unsupported schema {}, expected {CONFIG_SCHEMA}",
                config.schema
            ),
        });
    }
    let mut current = state.lock();
    *current = config;
    persist_config(&state.paths.config_file(), &current)?;
    Ok(current.clone())
}
