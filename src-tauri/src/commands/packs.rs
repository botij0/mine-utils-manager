use std::path::{Path, PathBuf};
use std::sync::Arc;

use mine_utils_core::{
    import_pack, resource_stack, scan_packs, AppError, HashCache, IconCache, ImportChoice,
    ImportOutcome, PackListing, SystemTrash,
};
use tauri::State;

use super::game_dirs::AppState;

/// Pack commands share the config store but keep the icon cache next to the
/// app cache dir, separate from the game dir. The hash cache is shared the same
/// way, so imports do not re-read every pack on each duplicate check.
pub struct PackState {
    icons: Arc<IconCache>,
    hashes: Arc<HashCache>,
}

impl PackState {
    pub fn new(cache_dir: &Path) -> Self {
        PackState {
            icons: Arc::new(IconCache::new(cache_dir)),
            hashes: Arc::new(HashCache::new()),
        }
    }

    pub fn icons(&self) -> Arc<IconCache> {
        Arc::clone(&self.icons)
    }

    pub fn hashes(&self) -> Arc<HashCache> {
        Arc::clone(&self.hashes)
    }
}

fn resourcepacks(state: &AppState) -> Result<PathBuf, AppError> {
    let game_dir = state.active_game_dir().ok_or(AppError::NoActiveGameDir)?;
    Ok(game_dir.join("resourcepacks"))
}

fn join_error(err: impl std::error::Error + Send + Sync + 'static) -> AppError {
    AppError::Io(std::io::Error::other(err))
}

/// Scanning reads files, so it runs on a blocking thread instead of the UI
/// thread the command is invoked on.
#[tauri::command]
pub async fn list_resource_packs(
    state: State<'_, AppState>,
    packs: State<'_, PackState>,
) -> Result<PackListing, AppError> {
    state.note_ipc();
    let root = resourcepacks(&state)?;
    let icons = packs.icons();
    tauri::async_runtime::spawn_blocking(move || scan_packs(&root, &icons))
        .await
        .map_err(join_error)?
}

#[tauri::command]
pub async fn import_resource_pack(
    state: State<'_, AppState>,
    packs: State<'_, PackState>,
    path: String,
    choice: Option<ImportChoice>,
) -> Result<ImportOutcome, AppError> {
    state.note_ipc();
    let root = resourcepacks(&state)?;
    let icons = packs.icons();
    let hashes = packs.hashes();
    tauri::async_runtime::spawn_blocking(move || {
        import_pack(
            Path::new(&path),
            &root,
            &icons,
            choice.unwrap_or_default(),
            &hashes,
        )
    })
    .await
    .map_err(join_error)?
}

/// Trash a pack and remove its `options.txt` references under the stack mutex.
#[tauri::command]
pub async fn delete_resource_pack(
    state: State<'_, AppState>,
    stack: State<'_, super::resource_stack::StackState>,
    path: String,
) -> Result<(), AppError> {
    state.note_ipc();
    // One config read decides the root and the stack paths, so a concurrent
    // active-dir change cannot split the delete across two game dirs.
    let stack_paths = state.stack_paths()?;
    let guard = stack.lock_for(&stack_paths.game_dir);
    tauri::async_runtime::spawn_blocking(move || {
        let _write = guard.lock().unwrap_or_else(|err| err.into_inner());
        resource_stack::delete_pack_safely(&stack_paths, Path::new(&path), &SystemTrash).map(|_| ())
    })
    .await
    .map_err(join_error)?
}
