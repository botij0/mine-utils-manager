use std::path::{Path, PathBuf};

use mine_utils_core::{
    delete_pack, import_pack, scan_packs, AppError, IconCache, ImportChoice, ImportOutcome,
    PackListing,
};
use tauri::State;

use super::game_dirs::AppState;

/// Pack commands share the config store but keep the icon cache next to the
/// app cache dir, separate from the game dir.
pub struct PackState {
    icons: IconCache,
}

impl PackState {
    pub fn new(cache_dir: &Path) -> Self {
        PackState {
            icons: IconCache::new(cache_dir),
        }
    }

    pub fn icons(&self) -> &IconCache {
        &self.icons
    }
}

fn resourcepacks(state: &AppState) -> Result<PathBuf, AppError> {
    let game_dir = state.active_game_dir().ok_or(AppError::NoActiveGameDir)?;
    Ok(game_dir.join("resourcepacks"))
}

#[tauri::command]
pub fn list_resource_packs(
    state: State<'_, AppState>,
    packs: State<'_, PackState>,
) -> Result<PackListing, AppError> {
    state.note_ipc();
    let root = resourcepacks(&state)?;
    scan_packs(&root, packs.icons())
}

#[tauri::command]
pub fn import_resource_pack(
    state: State<'_, AppState>,
    packs: State<'_, PackState>,
    path: String,
    choice: Option<ImportChoice>,
) -> Result<ImportOutcome, AppError> {
    state.note_ipc();
    let root = resourcepacks(&state)?;
    import_pack(
        Path::new(&path),
        &root,
        packs.icons(),
        choice.unwrap_or_default(),
    )
}

#[tauri::command]
pub fn delete_resource_pack(state: State<'_, AppState>, path: String) -> Result<(), AppError> {
    state.note_ipc();
    let root = resourcepacks(&state)?;
    delete_pack(Path::new(&path), &root)
}
