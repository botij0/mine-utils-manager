use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use mine_utils_core::{backup, resource_stack, AppError, BackupInfo, ResourceStack, StackSnapshot};
use tauri::State;

use super::game_dirs::AppState;

/// One write mutex per active game dir. Every `options.txt` write, restore,
/// and delete cleanup takes the mutex for its game dir, so no two writes ever
/// race inside the app. Reads do not take it.
#[derive(Default)]
pub struct StackState {
    locks: Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>,
}

impl StackState {
    pub fn lock_for(&self, game_dir: &Path) -> Arc<Mutex<()>> {
        let mut locks = self.locks.lock().unwrap_or_else(|err| err.into_inner());
        Arc::clone(locks.entry(game_dir.to_path_buf()).or_default())
    }
}

fn join_error(err: impl std::error::Error + Send + Sync + 'static) -> AppError {
    AppError::Io(std::io::Error::other(err))
}

/// The whole stack. Reads never block, even while the game runs.
#[tauri::command]
pub async fn get_resource_stack(state: State<'_, AppState>) -> Result<ResourceStack, AppError> {
    state.note_ipc();
    let paths = state.stack_paths()?;
    tauri::async_runtime::spawn_blocking(move || resource_stack::load(&paths))
        .await
        .map_err(join_error)?
}

/// Enable or disable one pack by its file name. The mutation is applied to the
/// latest `options.txt` content, and the refreshed stack comes back.
#[tauri::command]
pub async fn set_resource_pack_enabled(
    state: State<'_, AppState>,
    stack: State<'_, StackState>,
    filename: String,
    enabled: bool,
) -> Result<ResourceStack, AppError> {
    state.note_ipc();
    let paths = state.stack_paths()?;
    let guard = stack.lock_for(&paths.game_dir);
    tauri::async_runtime::spawn_blocking(move || {
        let _write = guard.lock().unwrap_or_else(|err| err.into_inner());
        resource_stack::set_enabled(&paths, &filename, enabled)
    })
    .await
    .map_err(join_error)?
}

/// Reorder the stack to `order`, load order low to high. `expected` is the
/// stack the UI rendered; a request against a changed file set comes back as
/// `stale_stack` so the UI refreshes instead of clobbering.
#[tauri::command]
pub async fn reorder_resource_stack(
    state: State<'_, AppState>,
    stack: State<'_, StackState>,
    order: Vec<String>,
    expected: StackSnapshot,
) -> Result<ResourceStack, AppError> {
    state.note_ipc();
    let paths = state.stack_paths()?;
    let guard = stack.lock_for(&paths.game_dir);
    tauri::async_runtime::spawn_blocking(move || {
        let _write = guard.lock().unwrap_or_else(|err| err.into_inner());
        resource_stack::reorder_stack(&paths, &order, &expected)
    })
    .await
    .map_err(join_error)?
}

/// List the `options.txt` backups of the active game dir, newest first.
#[tauri::command]
pub async fn list_options_backups(state: State<'_, AppState>) -> Result<Vec<BackupInfo>, AppError> {
    state.note_ipc();
    let paths = state.stack_paths()?;
    tauri::async_runtime::spawn_blocking(move || backup::list(&paths.backups_root))
        .await
        .map_err(join_error)?
}

/// Restore one backup over `options.txt` and return the refreshed stack.
#[tauri::command]
pub async fn restore_options_backup(
    state: State<'_, AppState>,
    stack: State<'_, StackState>,
    backup: String,
) -> Result<ResourceStack, AppError> {
    state.note_ipc();
    let paths = state.stack_paths()?;
    let guard = stack.lock_for(&paths.game_dir);
    tauri::async_runtime::spawn_blocking(move || {
        let _write = guard.lock().unwrap_or_else(|err| err.into_inner());
        resource_stack::restore_backup(&paths, &backup)
    })
    .await
    .map_err(join_error)?
}
