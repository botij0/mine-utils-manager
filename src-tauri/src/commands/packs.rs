use std::path::{Path, PathBuf};
use std::sync::Arc;

use mine_utils_core::{
    delete_pack, import_pack, scan_packs, AppError, HashCache, IconCache, ImportChoice,
    ImportOutcome, PackListing,
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

#[tauri::command]
pub async fn delete_resource_pack(
    state: State<'_, AppState>,
    path: String,
) -> Result<(), AppError> {
    state.note_ipc();
    let root = resourcepacks(&state)?;
    tauri::async_runtime::spawn_blocking(move || delete_pack(Path::new(&path), &root))
        .await
        .map_err(join_error)?
}
