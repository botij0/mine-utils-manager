//! Modrinth discovery and install. No request fires before the Discover
//! tab calls in: search runs only from the search page, downloads only from
//! `https://cdn.modrinth.com`, and every write goes through the existing
//! pack/datapack import paths so duplicates, locks, and guards behave as one.

use mine_utils_core::download::InstallControl;
use mine_utils_core::install::prepare_modrinth_install_controlled;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use mine_utils_core::{
    finish_modrinth_install, installed_game_versions, parse_install_kind,
    preferred_launcher_version, world, AppError, CoreInstallOutcome, DatapackImportOutcome,
    ImportOutcome, ModrinthClient, ModrinthError, ModrinthProject, ModrinthVersion,
    ReqwestTransport, SearchHit, SearchQuery, SearchSort,
};
use tauri::State;

use super::game_dirs::AppState;
use super::join_error;
use super::packs::PackState;
use super::resource_stack::StackState;

/// One shared HTTP client. `Arc` lets every command clone it into a blocking
/// thread without rebuilding connection pools.
pub struct ModrinthState {
    client: Arc<ModrinthClient<ReqwestTransport>>,
    installs: Arc<Mutex<HashMap<String, RegisteredInstall>>>,
}

impl ModrinthState {
    pub fn new() -> Self {
        ModrinthState {
            client: Arc::new(ModrinthClient::new(ReqwestTransport::default())),
            installs: Arc::default(),
        }
    }
}

impl Default for ModrinthState {
    fn default() -> Self {
        ModrinthState::new()
    }
}

enum RegisteredInstall {
    Ready(InstallControl),
    Running(InstallControl),
}

impl RegisteredInstall {
    fn control(&self) -> &InstallControl {
        match self {
            Self::Ready(control) | Self::Running(control) => control,
        }
    }
}

const MAX_REGISTERED_INSTALLS: usize = 64;

fn expire_ready(installs: &mut HashMap<String, RegisteredInstall>) {
    installs.retain(|_, entry| match entry {
        RegisteredInstall::Ready(control) => control.check().is_ok(),
        RegisteredInstall::Running(_) => true,
    });
}

#[tauri::command]
pub fn begin_modrinth_install(
    mods: State<'_, ModrinthState>,
    install_id: String,
) -> Result<(), AppError> {
    let mut installs = mods.installs.lock().unwrap_or_else(|err| err.into_inner());
    expire_ready(&mut installs);
    if installs.len() >= MAX_REGISTERED_INSTALLS {
        return Err(ModrinthError::Parse {
            reason: "too many active installs; cancel an install or wait for it to finish".into(),
        }
        .into());
    }
    if installs.contains_key(&install_id) {
        return Err(ModrinthError::Parse {
            reason: "install id is already active".to_string(),
        }
        .into());
    }
    installs.insert(
        install_id,
        RegisteredInstall::Ready(InstallControl::default()),
    );
    Ok(())
}

struct InstallRegistration {
    id: String,
    installs: Arc<Mutex<HashMap<String, RegisteredInstall>>>,
}

impl Drop for InstallRegistration {
    fn drop(&mut self) {
        self.installs
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .remove(&self.id);
    }
}

#[tauri::command]
pub fn cancel_modrinth_install(mods: State<'_, ModrinthState>, install_id: String) -> bool {
    let mut installs = mods.installs.lock().unwrap_or_else(|err| err.into_inner());
    let cancelled = installs
        .get(&install_id)
        .is_some_and(|entry| entry.control().cancel());
    if matches!(installs.get(&install_id), Some(RegisteredInstall::Ready(_))) {
        installs.remove(&install_id);
    }
    expire_ready(&mut installs);
    cancelled
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

const SEARCH_PAGE_SIZE: u8 = 20;

/// One Discover search page. `has_more` keeps the 20-hit page size in the
/// backend, so the frontend does not mirror it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchPage {
    pub hits: Vec<SearchHit>,
    pub has_more: bool,
}

impl SearchPage {
    fn new(hits: Vec<SearchHit>) -> Self {
        SearchPage {
            has_more: hits.len() == usize::from(SEARCH_PAGE_SIZE),
            hits,
        }
    }
}

fn build_search_query(
    text: String,
    kind: String,
    game_version: Option<String>,
    sort: Option<String>,
    offset: u32,
) -> Result<SearchQuery, AppError> {
    Ok(SearchQuery {
        text,
        kind: parse_install_kind(&kind)?,
        game_version,
        sort: parse_sort(sort.as_deref().unwrap_or("relevance"))?,
        limit: SEARCH_PAGE_SIZE,
        offset,
    })
}

/// Search Modrinth. Runs only when the Discover page calls it; nothing
/// prefetches, and the local tabs never trigger a request. One page of 20
/// hits per call; the offset selects the page for an explicit Load more.
#[tauri::command]
pub async fn search_modrinth(
    state: State<'_, AppState>,
    mods: State<'_, ModrinthState>,
    query: String,
    kind: String,
    game_version: Option<String>,
    sort: Option<String>,
    offset: Option<u32>,
) -> Result<SearchPage, AppError> {
    state.note_ipc();
    let client = Arc::clone(&mods.client);
    tauri::async_runtime::spawn_blocking(move || {
        let query = build_search_query(query, kind, game_version, sort, offset.unwrap_or(0))?;
        Ok(SearchPage::new(client.search(&query)?))
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
/// or a locked world keeps the pack disabled. The wire kind is `resourcepack`
/// (one word, matching the Modrinth project type and the TS union), not the
/// default `resource_pack` snake_case. `provenance_warning` is set when the
/// file installed fine but the provenance record write failed; it never
/// fails the install.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModrinthInstallOutcome {
    #[serde(rename = "resourcepack")]
    ResourcePack {
        outcome: ImportOutcome,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provenance_warning: Option<String>,
    },
    Datapack {
        outcome: DatapackImportOutcome,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provenance_warning: Option<String>,
    },
}

impl From<CoreInstallOutcome> for ModrinthInstallOutcome {
    fn from(outcome: CoreInstallOutcome) -> Self {
        match outcome {
            CoreInstallOutcome::ResourcePack {
                outcome,
                provenance_warning,
            } => ModrinthInstallOutcome::ResourcePack {
                outcome,
                provenance_warning,
            },
            CoreInstallOutcome::Datapack {
                outcome,
                provenance_warning,
            } => ModrinthInstallOutcome::Datapack {
                outcome,
                provenance_warning,
            },
        }
    }
}

/// Download a version file from the CDN, verify it, commit it under the API
/// filename, and route it through the existing import paths so duplicates,
/// collisions, locks, and the game guard behave exactly like a manual
/// import. The metadata fetch and download run with no game lock held; only
/// the commit/import/enable/provenance phase takes the per-game-dir lock.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn install_modrinth_pack(
    state: State<'_, AppState>,
    packs: State<'_, PackState>,
    stack: State<'_, StackState>,
    mods: State<'_, ModrinthState>,
    install_id: String,
    project_id: String,
    version_id: String,
    filename: String,
    kind: String,
    world: Option<String>,
) -> Result<ModrinthInstallOutcome, AppError> {
    state.note_ipc();
    let control = {
        let mut installs = mods.installs.lock().unwrap_or_else(|err| err.into_inner());
        match installs.get_mut(&install_id) {
            Some(entry @ RegisteredInstall::Ready(_)) => {
                let control = entry.control().clone();
                *entry = RegisteredInstall::Running(control.clone());
                control
            }
            None => return Err(mine_utils_core::download::DownloadError::Cancelled.into()),
            Some(RegisteredInstall::Running(_)) => {
                return Err(ModrinthError::Parse {
                    reason: "install id is not ready".to_string(),
                }
                .into())
            }
        }
    };
    let registration = InstallRegistration {
        id: install_id,
        installs: Arc::clone(&mods.installs),
    };
    // One snapshot: the game dir, stack paths, and backup policy are read
    // once, not three times across the install.
    let snapshot = state.snapshot()?;
    let stack_paths = snapshot.stack_paths();
    let data_paths = match world.clone() {
        Some(name) => Some(snapshot.world_data_paths(&name)?),
        None => None,
    };
    let guard = stack.lock_for(&snapshot.game_dir);
    let data_dir = snapshot.data_dir.clone();
    let icons = packs.icons();
    let hashes = packs.hashes();
    let client = Arc::clone(&mods.client);
    tauri::async_runtime::spawn_blocking(move || {
        let _registration = registration;
        let prepared = prepare_modrinth_install_controlled(
            &client,
            &snapshot.game_dir,
            world.as_deref(),
            &project_id,
            &version_id,
            &filename,
            &kind,
            &control,
        )?;
        let _write = guard.lock().unwrap_or_else(|err| err.into_inner());
        finish_modrinth_install(
            prepared,
            &stack_paths,
            data_paths.as_ref(),
            &data_dir,
            &icons,
            &hashes,
        )
        .map(ModrinthInstallOutcome::from)
    })
    .await
    .map_err(join_error)?
}

#[cfg(test)]
mod tests {
    use super::*;

    use tauri::test::{mock_builder, mock_context, noop_assets};
    use tauri::Manager;

    #[test]
    fn unused_registrations_are_released_on_cancel() {
        let app = mock_builder()
            .manage(ModrinthState::new())
            .build(mock_context(noop_assets()))
            .unwrap();
        for _ in 0..100 {
            begin_modrinth_install(app.state(), "unused".into()).unwrap();
            assert!(cancel_modrinth_install(app.state(), "unused".into()));
            let count = app.state::<ModrinthState>().installs.lock().unwrap().len();
            assert_eq!(count, 0, "cancel must release a Ready registration");
        }
        begin_modrinth_install(app.state(), "unused".into()).unwrap();
        assert_eq!(
            app.state::<ModrinthState>().installs.lock().unwrap().len(),
            1
        );
        println!(
            "registry replay: 100 register/cancel pairs left 0 entries; same id registers again"
        );
    }

    #[test]
    fn expired_ready_registration_can_be_registered_again() {
        let app = mock_builder()
            .manage(ModrinthState::new())
            .build(mock_context(noop_assets()))
            .unwrap();
        app.state::<ModrinthState>()
            .installs
            .lock()
            .unwrap()
            .insert(
                "expired".into(),
                RegisteredInstall::Ready(InstallControl::with_timeout(std::time::Duration::ZERO)),
            );
        begin_modrinth_install(app.state(), "expired".into()).unwrap();
        assert_eq!(
            app.state::<ModrinthState>().installs.lock().unwrap().len(),
            1
        );
        assert!(cancel_modrinth_install(app.state(), "expired".into()));
    }

    #[test]
    fn unused_registrations_have_a_capacity_limit() {
        let app = mock_builder()
            .manage(ModrinthState::new())
            .build(mock_context(noop_assets()))
            .unwrap();
        let accepted = (0..100)
            .filter(|id| begin_modrinth_install(app.state(), format!("unused-{id}")).is_ok())
            .count();
        assert_eq!(accepted, 64);
        assert_eq!(
            app.state::<ModrinthState>().installs.lock().unwrap().len(),
            64
        );
        assert!(cancel_modrinth_install(app.state(), "unused-0".into()));
        begin_modrinth_install(app.state(), "replacement".into()).unwrap();
        println!(
            "registry replay: 100 abandoned registrations retained 64; cancellation freed a slot"
        );
    }

    fn hit() -> SearchHit {
        serde_json::from_value(serde_json::json!({ "project_id": "p" })).expect("hit")
    }

    #[test]
    fn search_page_reports_more_only_when_the_page_is_full() {
        let full = SearchPage::new((0..20).map(|_| hit()).collect());
        assert!(full.has_more);
        let wire = serde_json::to_value(&full).expect("serializes");
        assert_eq!(
            wire.get("has_more").and_then(|value| value.as_bool()),
            Some(true)
        );

        let short = SearchPage::new((0..19).map(|_| hit()).collect());
        assert!(!short.has_more);
        assert!(!SearchPage::new(Vec::new()).has_more);
    }

    #[test]
    fn search_query_forwards_offset_and_keeps_the_page_size() {
        let query = build_search_query(
            "fresh".to_string(),
            "resourcepack".to_string(),
            Some("1.21.1".to_string()),
            Some("downloads".to_string()),
            40,
        )
        .expect("query");

        assert_eq!(query.offset, 40);
        assert_eq!(query.limit, 20);
        assert!(matches!(query.sort, SearchSort::Downloads));
        assert_eq!(query.game_version.as_deref(), Some("1.21.1"));
    }

    /// Locks the wire contract that caused the original `Installed undefined`
    /// bug: the Tauri command serializes the outcome kind as `resourcepack`
    /// (one word, matching the Modrinth project type), never `resource_pack`.
    #[test]
    fn install_outcome_kind_is_one_word_on_the_wire() {
        let resource = ModrinthInstallOutcome::ResourcePack {
            outcome: ImportOutcome::Skipped,
            provenance_warning: None,
        };
        let wire = serde_json::to_value(&resource).expect("serializes");
        assert_eq!(
            wire.get("kind").and_then(|k| k.as_str()),
            Some("resourcepack")
        );

        let data = ModrinthInstallOutcome::Datapack {
            outcome: DatapackImportOutcome::Skipped,
            provenance_warning: Some("w".to_string()),
        };
        let wire = serde_json::to_value(&data).expect("serializes");
        assert_eq!(wire.get("kind").and_then(|k| k.as_str()), Some("datapack"));
        // A `resource_pack` payload must never silently render as a datapack.
        let legacy: Result<ModrinthInstallOutcome, _> =
            serde_json::from_str(r#"{"kind":"resource_pack","outcome":{"status":"skipped"}}"#);
        assert!(legacy.is_err(), "snake_case kind must not deserialize");
    }
}
