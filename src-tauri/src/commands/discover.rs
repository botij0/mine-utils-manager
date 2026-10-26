//! Modrinth discovery and install. No request fires before the Discover
//! tab calls in: search runs only from the search page, downloads only from
//! `https://cdn.modrinth.com`, and every write goes through the existing
//! pack/datapack import paths so duplicates, locks, and guards behave as one.

use std::path::PathBuf;
use std::sync::Arc;

use mine_utils_core::{
    data_stack, download_verified, installed_game_versions, preferred_launcher_version,
    primary_file, provenance_record, resource_stack, world, AppError, DatapackImportOutcome,
    ImportOutcome, ModrinthClient, ModrinthError, ModrinthProject, ModrinthVersion,
    ProvenanceEntry, ReqwestTransport, SearchHit, SearchKind, SearchQuery, SearchSort, ZipRoot,
};
use tauri::State;

use super::game_dirs::AppState;
use super::packs::PackState;
use super::resource_stack::StackState;

/// One shared HTTP client. `Arc` lets every command clone it into a blocking
/// thread without rebuilding connection pools.
pub struct ModrinthState {
    client: Arc<ModrinthClient<ReqwestTransport>>,
}

impl ModrinthState {
    pub fn new() -> Self {
        ModrinthState {
            client: Arc::new(ModrinthClient::new(ReqwestTransport::default())),
        }
    }
}

impl Default for ModrinthState {
    fn default() -> Self {
        ModrinthState::new()
    }
}

fn join_error(err: impl std::error::Error + Send + Sync + 'static) -> AppError {
    AppError::Io(std::io::Error::other(err))
}

fn parse_kind(kind: &str) -> Result<SearchKind, ModrinthError> {
    match kind {
        "resourcepack" => Ok(SearchKind::ResourcePack),
        "datapack" => Ok(SearchKind::Datapack),
        other => Err(ModrinthError::Parse {
            reason: format!("unknown project type {other:?}; want resourcepack or datapack"),
        }),
    }
}

fn parse_sort(sort: &str) -> Result<SearchSort, ModrinthError> {
    match sort {
        "relevance" => Ok(SearchSort::Relevance),
        "downloads" => Ok(SearchSort::Downloads),
        "follows" => Ok(SearchSort::Follows),
        "newest" => Ok(SearchSort::Newest),
        "updated" => Ok(SearchSort::Updated),
        other => Err(ModrinthError::Parse {
            reason: format!("unknown sort {other:?}"),
        }),
    }
}

/// Search Modrinth. Runs only when the Discover page calls it; nothing
/// prefetches, and the local tabs never trigger a request.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn search_modrinth(
    state: State<'_, AppState>,
    mods: State<'_, ModrinthState>,
    query: String,
    kind: String,
    game_version: Option<String>,
    sort: Option<String>,
    limit: Option<u8>,
    offset: Option<u32>,
) -> Result<Vec<SearchHit>, AppError> {
    state.note_ipc();
    let client = Arc::clone(&mods.client);
    tauri::async_runtime::spawn_blocking(move || {
        let found = client.search(&SearchQuery {
            text: query,
            kind: parse_kind(&kind)?,
            game_version,
            sort: parse_sort(sort.as_deref().unwrap_or("relevance"))?,
            limit: limit.unwrap_or(20),
            offset: offset.unwrap_or(0),
        })?;
        Ok(found)
    })
    .await
    .map_err(join_error)?
}

/// Project metadata for the detail sheet.
#[tauri::command]
pub async fn get_modrinth_project(
    state: State<'_, AppState>,
    mods: State<'_, ModrinthState>,
    project_id: String,
) -> Result<ModrinthProject, AppError> {
    state.note_ipc();
    let client = Arc::clone(&mods.client);
    tauri::async_runtime::spawn_blocking(move || {
        client.project(&project_id).map_err(AppError::from)
    })
    .await
    .map_err(join_error)?
}
/// Every version of a project, newest first. The picker shows all of them;
// the UI preselects the compatible one.
#[tauri::command]
pub async fn list_modrinth_versions(
    state: State<'_, AppState>,
    mods: State<'_, ModrinthState>,
    project_id: String,
) -> Result<Vec<ModrinthVersion>, AppError> {
    state.note_ipc();
    let client = Arc::clone(&mods.client);
    tauri::async_runtime::spawn_blocking(move || {
        client.versions(&project_id).map_err(AppError::from)
    })
    .await
    .map_err(join_error)?
}

/// What the Discover install sheet offers as defaults: the installed game
/// versions, the launcher's selected version, and the world's version for
/// datapacks. Nothing here is a filter; the picker lists every version.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ModrinthTargets {
    pub installed_versions: Vec<String>,
    pub preferred_launcher: Option<String>,
    pub world_version: Option<String>,
}

#[tauri::command]
pub async fn modrinth_targets(
    state: State<'_, AppState>,
    packs: State<'_, PackState>,
    world: Option<String>,
) -> Result<ModrinthTargets, AppError> {
    state.note_ipc();
    let game_dir = state.active_game_dir().ok_or(AppError::NoActiveGameDir)?;
    let icons = packs.icons();
    tauri::async_runtime::spawn_blocking(move || {
        let installed_versions = installed_game_versions(&game_dir);
        let preferred_launcher = preferred_launcher_version(&game_dir);
        let world_version = world
            .map(|name| {
                world::scan_worlds(&game_dir, &icons)
                    .into_iter()
                    .find(|w| w.dir_name == name)
                    .and_then(|w| w.version.map(|v| v.name))
            })
            .unwrap_or(None);
        Ok(ModrinthTargets {
            installed_versions,
            preferred_launcher,
            world_version,
        })
    })
    .await
    .map_err(join_error)?
}

/// What an install did. Resource outcomes match the Packs page; datapack
/// outcomes match the Worlds page, including `locked` when a running game
/// or a locked world keeps the pack disabled.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModrinthInstallOutcome {
    ResourcePack { outcome: ImportOutcome },
    Datapack { outcome: DatapackImportOutcome },
}

/// Download a version file from the CDN, verify it, and route it through the
/// existing import paths so duplicates, collisions, locks, and the game
/// guard behave exactly like a manual import.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn install_modrinth_pack(
    state: State<'_, AppState>,
    packs: State<'_, PackState>,
    stack: State<'_, StackState>,
    mods: State<'_, ModrinthState>,
    project_id: String,
    version_id: String,
    filename: String,
    kind: String,
    world: Option<String>,
) -> Result<ModrinthInstallOutcome, AppError> {
    state.note_ipc();
    let game_dir = state.active_game_dir().ok_or(AppError::NoActiveGameDir)?;
    let stack_paths = state.stack_paths()?;
    let data_paths = match world.clone() {
        Some(name) => Some(state.world_data_paths(&name)?),
        None => None,
    };
    let data_dir = state.data_dir();
    let icons = packs.icons();
    let hashes = packs.hashes();
    let guard = stack.lock_for(&stack_paths.game_dir);
    let client = Arc::clone(&mods.client);
    tauri::async_runtime::spawn_blocking(move || {
        let _write = guard.lock().unwrap_or_else(|err| err.into_inner());
        install_blocking(
            &client,
            &game_dir,
            &stack_paths,
            data_paths.as_ref(),
            &data_dir,
            &icons,
            &hashes,
            &project_id,
            &version_id,
            &filename,
            &kind,
        )
    })
    .await
    .map_err(join_error)?
}

#[allow(clippy::too_many_arguments)]
fn install_blocking(
    client: &ModrinthClient<ReqwestTransport>,
    game_dir: &std::path::Path,
    stack_paths: &mine_utils_core::StackPaths,
    data_paths: Option<&mine_utils_core::DataPaths>,
    data_dir: &std::path::Path,
    icons: &mine_utils_core::IconCache,
    hashes: &mine_utils_core::HashCache,
    project_id: &str,
    version_id: &str,
    filename: &str,
    kind: &str,
) -> Result<ModrinthInstallOutcome, AppError> {
    let search_kind = parse_kind(kind)?;
    let versions = client.versions(project_id)?;
    let version = versions
        .iter()
        .find(|v| v.id == version_id)
        .ok_or_else(|| ModrinthError::Parse {
            reason: format!("version {version_id} is not in project {project_id}"),
        })?;
    let file = version
        .files
        .iter()
        .find(|f| f.filename == filename)
        .or_else(|| primary_file(version))
        .ok_or_else(|| ModrinthError::Parse {
            reason: format!("version {version_id} has no downloadable file"),
        })?;
    if file.filename != filename {
        return Err(ModrinthError::Parse {
            reason: format!(
                "version {version_id} has no file named {filename:?}; closest is {:?}",
                file.filename
            ),
        }
        .into());
    }
    let root = match search_kind {
        SearchKind::ResourcePack => ZipRoot::Assets,
        SearchKind::Datapack => ZipRoot::Data,
    };
    let target_dir: PathBuf = match search_kind {
        SearchKind::ResourcePack => game_dir.join("resourcepacks"),
        SearchKind::Datapack => {
            let paths = data_paths.ok_or_else(|| ModrinthError::Parse {
                reason: "a datapack install needs a target world".to_string(),
            })?;
            paths.datapacks_dir.clone()
        }
    };
    let verified = download_verified(
        client,
        &file.url,
        &target_dir,
        &file.filename,
        &file.hashes.sha512,
        root,
    )?;
    let outcome = route_verified(
        &verified.temp_path,
        stack_paths,
        data_paths,
        icons,
        hashes,
        &file.filename,
        search_kind,
    );
    let _ = std::fs::remove_file(&verified.temp_path);
    let outcome = outcome?;
    if installed(&outcome) {
        provenance_record(
            data_dir,
            &verified.sha256,
            ProvenanceEntry {
                project_id: project_id.to_string(),
                version_id: version_id.to_string(),
                file_name: file.filename.clone(),
            },
        )?;
    }
    Ok(outcome)
}

/// Hand the verified temp to the existing import paths, then enable through
/// the stack paths. A running game blocks `options.txt`/`level.dat` writes
/// through the existing guards; a locked world copies but stays disabled.
fn route_verified(
    temp: &std::path::Path,
    stack_paths: &mine_utils_core::StackPaths,
    data_paths: Option<&mine_utils_core::DataPaths>,
    icons: &mine_utils_core::IconCache,
    hashes: &mine_utils_core::HashCache,
    filename: &str,
    kind: SearchKind,
) -> Result<ModrinthInstallOutcome, AppError> {
    match kind {
        SearchKind::ResourcePack => {
            let outcome = mine_utils_core::import_pack(
                temp,
                &stack_paths.game_dir.join("resourcepacks"),
                icons,
                mine_utils_core::ImportChoice::Default,
                hashes,
            )?;
            if let ImportOutcome::Imported { .. } = &outcome {
                resource_stack::set_enabled(stack_paths, filename, true)?;
            }
            Ok(ModrinthInstallOutcome::ResourcePack { outcome })
        }
        SearchKind::Datapack => {
            let paths = data_paths.expect("datapack target checked above");
            let outcome = data_stack::import_datapack(
                paths,
                temp,
                mine_utils_core::ImportChoice::Default,
                icons,
                hashes,
            )?;
            Ok(ModrinthInstallOutcome::Datapack { outcome })
        }
    }
}

fn installed(outcome: &ModrinthInstallOutcome) -> bool {
    match outcome {
        ModrinthInstallOutcome::ResourcePack { outcome } => {
            matches!(outcome, ImportOutcome::Imported { .. })
        }
        ModrinthInstallOutcome::Datapack { outcome } => matches!(
            outcome,
            DatapackImportOutcome::Imported { .. } | DatapackImportOutcome::Locked { .. }
        ),
    }
}

/// Where an installed file came from, if it was installed from Modrinth.
#[tauri::command]
pub async fn get_modrinth_provenance(
    state: State<'_, AppState>,
    sha256: String,
) -> Result<Option<ProvenanceEntry>, AppError> {
    state.note_ipc();
    let data_dir = state.data_dir();
    tauri::async_runtime::spawn_blocking(move || {
        mine_utils_core::provenance_lookup(&data_dir, &sha256).map_err(AppError::from)
    })
    .await
    .map_err(join_error)?
}
