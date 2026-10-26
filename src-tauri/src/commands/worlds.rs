use std::path::Path;

use mine_utils_core::{
    data_stack, level_backup, world, AppError, DataSnapshot, DataStack, DatapackImportOutcome,
    ImportChoice, LevelBackupInfo, PackListing, SystemTrash, World,
};
use tauri::State;

use super::game_dirs::AppState;
use super::packs::PackState;
use super::resource_stack::StackState;

fn join_error(err: impl std::error::Error + Send + Sync + 'static) -> AppError {
    AppError::Io(std::io::Error::other(err))
}

/// Every world in the active game dir, with versions, icons, and lock state.
/// Reads never block, even while the game runs.
#[tauri::command]
pub async fn list_worlds(
    state: State<'_, AppState>,
    packs: State<'_, PackState>,
) -> Result<Vec<World>, AppError> {
    state.note_ipc();
    let game_dir = state.active_game_dir().ok_or(AppError::NoActiveGameDir)?;
    let icons = packs.icons();
    tauri::async_runtime::spawn_blocking(move || world::scan_worlds(&game_dir, &icons))
        .await
        .map_err(join_error)
}

/// The installed datapacks of one world plus its enabled/disabled stack.
#[tauri::command]
pub async fn list_datapacks(
    state: State<'_, AppState>,
    packs: State<'_, PackState>,
    world: String,
) -> Result<DatapackListing, AppError> {
    state.note_ipc();
    let paths = state.world_data_paths(&world)?;
    let icons = packs.icons();
    tauri::async_runtime::spawn_blocking(move || {
        let packs = data_stack::scan_datapacks(&paths.datapacks_dir, &icons)?;
        let stack = data_stack::load(&paths)?;
        Ok(DatapackListing { packs, stack })
    })
    .await
    .map_err(join_error)?
}

/// The installed packs plus the stack, as the Worlds page renders them.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DatapackListing {
    pub packs: PackListing,
    pub stack: DataStack,
}

/// Import a zip or directory datapack into `saves/<world>/datapacks`. When
/// the world is locked the file is copied but left disabled.
#[tauri::command]
pub async fn import_datapack(
    state: State<'_, AppState>,
    packs: State<'_, PackState>,
    stack: State<'_, StackState>,
    world: String,
    path: String,
    choice: Option<ImportChoice>,
) -> Result<DatapackImportOutcome, AppError> {
    state.note_ipc();
    let paths = state.world_data_paths(&world)?;
    let icons = packs.icons();
    let hashes = packs.hashes();
    let guard = stack.lock_for(&paths.game_dir);
    tauri::async_runtime::spawn_blocking(move || {
        let _write = guard.lock().unwrap_or_else(|err| err.into_inner());
        data_stack::import_datapack(
            &paths,
            Path::new(&path),
            choice.unwrap_or_default(),
            &icons,
            &hashes,
        )
    })
    .await
    .map_err(join_error)?
}

/// Enable or disable one datapack by file name. Blocked while the game runs
/// or the world is locked.
#[tauri::command]
pub async fn set_datapack_enabled(
    state: State<'_, AppState>,
    stack: State<'_, StackState>,
    world: String,
    filename: String,
    enabled: bool,
) -> Result<DataStack, AppError> {
    state.note_ipc();
    let paths = state.world_data_paths(&world)?;
    let guard = stack.lock_for(&paths.game_dir);
    tauri::async_runtime::spawn_blocking(move || {
        let _write = guard.lock().unwrap_or_else(|err| err.into_inner());
        data_stack::set_enabled(&paths, &filename, enabled)
    })
    .await
    .map_err(join_error)?
}

/// Reorder the `Enabled` list to `order`, load order low to high. `expected`
/// is the stack the UI rendered; a request against a changed file set comes
/// back as stale so the UI refreshes.
#[tauri::command]
pub async fn reorder_datapacks(
    state: State<'_, AppState>,
    stack: State<'_, StackState>,
    world: String,
    order: Vec<String>,
    expected: DataSnapshot,
) -> Result<DataStack, AppError> {
    state.note_ipc();
    let paths = state.world_data_paths(&world)?;
    let guard = stack.lock_for(&paths.game_dir);
    tauri::async_runtime::spawn_blocking(move || {
        let _write = guard.lock().unwrap_or_else(|err| err.into_inner());
        data_stack::reorder_stack(&paths, &order, &expected)
    })
    .await
    .map_err(join_error)?
}

/// Delete one datapack: guard, clean both lists, guard again, trash. Returns
/// the refreshed stack.
#[tauri::command]
pub async fn delete_datapack(
    state: State<'_, AppState>,
    stack: State<'_, StackState>,
    world: String,
    path: String,
) -> Result<DatapackDeleteOutcome, AppError> {
    state.note_ipc();
    let paths = state.world_data_paths(&world)?;
    let guard = stack.lock_for(&paths.game_dir);
    tauri::async_runtime::spawn_blocking(move || {
        let _write = guard.lock().unwrap_or_else(|err| err.into_inner());
        let outcome = data_stack::delete_datapack_safely(&paths, Path::new(&path), &SystemTrash)?;
        let stack = data_stack::load(&paths)?;
        Ok(DatapackDeleteOutcome {
            removed_references: outcome.removed_references,
            stack,
        })
    })
    .await
    .map_err(join_error)?
}

/// What a delete did, plus the refreshed stack for the UI.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DatapackDeleteOutcome {
    pub removed_references: bool,
    pub stack: DataStack,
}

/// The `level.dat` backups of one world, newest first.
#[tauri::command]
pub async fn list_level_backups(
    state: State<'_, AppState>,
    world: String,
) -> Result<Vec<LevelBackupInfo>, AppError> {
    state.note_ipc();
    let paths = state.world_data_paths(&world)?;
    tauri::async_runtime::spawn_blocking(move || {
        level_backup::list(&paths.backups_root, &paths.world_name())
    })
    .await
    .map_err(join_error)?
}

/// Every `level.dat` backup in the game dir's folder, newest first, with the
/// world dir name on each row. The UI groups by world without one IPC call
/// per world, and corrupt or missing-file worlds still show their backups.
#[tauri::command]
pub async fn list_all_level_backups(
    state: State<'_, AppState>,
) -> Result<Vec<LevelBackupInfo>, AppError> {
    state.note_ipc();
    let root = state.level_backups_root()?;
    tauri::async_runtime::spawn_blocking(move || level_backup::list_all(&root))
        .await
        .map_err(join_error)?
}

/// Drop one dead `file/<filename>` reference from both datapack lists without
/// touching the disk pack (there is none). Used for entries whose file is
/// missing from `datapacks`; enabling those would only write a dead entry.
#[tauri::command]
pub async fn remove_datapack_reference(
    state: State<'_, AppState>,
    stack: State<'_, StackState>,
    world: String,
    filename: String,
) -> Result<DataStack, AppError> {
    state.note_ipc();
    let paths = state.world_data_paths(&world)?;
    let guard = stack.lock_for(&paths.game_dir);
    tauri::async_runtime::spawn_blocking(move || {
        let _write = guard.lock().unwrap_or_else(|err| err.into_inner());
        data_stack::remove_references(&paths, &filename)?;
        data_stack::load(&paths)
    })
    .await
    .map_err(join_error)?
}

/// Restore one backup over `level.dat` and return the refreshed stack.
#[tauri::command]
pub async fn restore_level_backup(
    state: State<'_, AppState>,
    stack: State<'_, StackState>,
    world: String,
    backup: String,
) -> Result<DataStack, AppError> {
    state.note_ipc();
    let paths = state.world_data_paths(&world)?;
    let guard = stack.lock_for(&paths.game_dir);
    tauri::async_runtime::spawn_blocking(move || {
        let _write = guard.lock().unwrap_or_else(|err| err.into_inner());
        data_stack::restore_backup(&paths, &backup)
    })
    .await
    .map_err(join_error)?
}
