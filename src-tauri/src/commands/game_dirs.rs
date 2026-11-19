pub use mine_utils_core::wire::GameDirState;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Once};

use super::join_error;

use mine_utils_core::{
    contains_entry, discover_game_dirs, entry_for, home_dir, is_game_dir, level_backup,
    merge_game_dirs, validate_game_dir, AppError, BackupsConfig, Config, ConfigStore, DataPaths,
    DiscoverySource, GameDirEntry, StackPaths, Theme, WorldPaths,
};
use tauri::State;

#[derive(Clone)]
struct ActiveSwitch {
    next: Arc<AtomicU64>,
    applied: Arc<AtomicU64>,
}

impl ActiveSwitch {
    fn new() -> Self {
        Self {
            next: Arc::new(AtomicU64::new(0)),
            applied: Arc::new(AtomicU64::new(0)),
        }
    }

    fn issue(&self) -> u64 {
        self.next.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn applied(&self) -> u64 {
        self.applied.load(Ordering::SeqCst)
    }

    fn commit(&self, ticket: u64) {
        self.applied.fetch_max(ticket, Ordering::SeqCst);
    }
}

#[derive(Clone)]
pub struct AppState {
    store: Arc<Mutex<ConfigStore>>,
    committed: Arc<Mutex<Config>>,
    startup_warning: Option<String>,
    first_ipc: Arc<Once>,
    data_dir: PathBuf,
    active_switch: ActiveSwitch,
}

impl AppState {
    pub fn new(store: ConfigStore, startup_warning: Option<String>, data_dir: PathBuf) -> Self {
        let committed = store.config().clone();
        AppState {
            store: Arc::new(Mutex::new(store)),
            committed: Arc::new(Mutex::new(committed)),
            startup_warning,
            first_ipc: Arc::new(Once::new()),
            data_dir,
            active_switch: ActiveSwitch::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ConfigStore> {
        self.store.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn config(&self) -> Config {
        self.committed
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    fn update<T>(
        &self,
        f: impl FnOnce(&mut ConfigStore) -> Result<T, AppError>,
    ) -> Result<(T, Config), AppError> {
        let mut store = self.lock();
        let result = f(&mut store)?;
        let config = store.config().clone();
        *self.committed.lock().unwrap_or_else(|err| err.into_inner()) = config.clone();
        Ok((result, config))
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
        self.config().active_game_dir
    }

    /// One consistent read of the active game dir and its backup policy.
    /// Every write command derives its paths from this snapshot, so a config
    /// change mid-command cannot split paths across two game dirs.
    pub fn snapshot(&self) -> Result<GameDirSnapshot, AppError> {
        let (game_dir, backups) = {
            let config = self.config();
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
            &resolved,
            &self.data_dir,
            self.backups.clone(),
        ))
    }
}

fn listing(config: &Config) -> Result<Vec<GameDirEntry>, AppError> {
    let home = home_dir()?;
    let merged = merge_game_dirs(discover_game_dirs(&home), &config.game_dirs);
    let stored: HashSet<PathBuf> = config
        .game_dirs
        .iter()
        .map(|dir| dir.path.clone())
        .collect();
    let mut entries: Vec<_> = merged
        .into_iter()
        .map(|dir| {
            let mut entry = entry_for(&dir);
            entry.stored = stored.contains(&dir.path);
            entry
        })
        .collect();
    if let Some(path) = config.active_game_dir.as_deref() {
        if !contains_entry(&entries, path) && is_game_dir(path) {
            entries.push(GameDirEntry {
                path: path.to_path_buf(),
                source: DiscoverySource::Manual,
                present: true,
                stored: false,
            });
        }
    }
    Ok(entries)
}

fn state_for(config: Config) -> Result<GameDirState, AppError> {
    let entries = listing(&config)?;
    Ok(GameDirState { config, entries })
}

#[tauri::command]
pub async fn list_game_dirs(state: State<'_, AppState>) -> Result<Vec<GameDirEntry>, AppError> {
    state.note_first_ipc();
    let config = state.config();
    tauri::async_runtime::spawn_blocking(move || listing(&config))
        .await
        .map_err(join_error)?
}

#[tauri::command]
pub async fn add_game_dir(
    state: State<'_, AppState>,
    path: String,
) -> Result<GameDirState, AppError> {
    state.note_first_ipc();
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let dir = validate_game_dir(Path::new(&path))?;
        let (_, config) = state.update(|store| store.add_game_dir(dir))?;
        state_for(config)
    })
    .await
    .map_err(join_error)?
}

#[tauri::command]
pub async fn set_last_world(state: State<'_, AppState>, path: String) -> Result<(), AppError> {
    state.note_first_ipc();
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let path = PathBuf::from(&path);
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string)
            .ok_or_else(|| AppError::World {
                reason: "invalid world path".into(),
            })?;
        state
            .update(|store| {
                let game = store
                    .config()
                    .active_game_dir
                    .as_deref()
                    .ok_or(AppError::NoActiveGameDir)?;
                if path != game.join("saves").join(&name) {
                    return Err(AppError::World {
                        reason: "world is not in the active game dir".into(),
                    });
                }
                store.set_last_world(&name)
            })
            .map(|_| ())
    })
    .await
    .map_err(join_error)?
}

#[tauri::command]
pub async fn remove_game_dir(
    state: State<'_, AppState>,
    path: String,
) -> Result<GameDirState, AppError> {
    state.note_first_ipc();
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let target =
            std::fs::canonicalize(Path::new(&path)).unwrap_or_else(|_| PathBuf::from(&path));
        let (_, config) = state.update(|store| store.remove_game_dir(&target))?;
        state_for(config)
    })
    .await
    .map_err(join_error)?
}

#[tauri::command]
pub async fn set_active_game_dir(
    state: State<'_, AppState>,
    path: String,
) -> Result<GameDirState, AppError> {
    state.note_first_ipc();
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        set_active_with(&state, &path, |path| {
            std::fs::canonicalize(path).map_err(|_| AppError::UnknownGameDir {
                path: path.to_path_buf(),
            })
        })
    })
    .await
    .map_err(join_error)?
}

fn set_active_with(
    state: &AppState,
    path: &str,
    canonicalize: impl FnOnce(&Path) -> Result<PathBuf, AppError>,
) -> Result<GameDirState, AppError> {
    let ticket = state.active_switch.issue();
    let target = canonicalize(Path::new(path))?;
    let discovered = discover_game_dirs(&home_dir()?);
    let found = discovered.iter().any(|dir| dir.path == target);
    let stored = state.lock().config().game_dirs.clone();
    let merged = merge_game_dirs(discovered, &stored);
    let entries: Vec<_> = merged.iter().map(entry_for).collect();
    let active_switch = state.active_switch.clone();
    let (_, config) = state.update(|store| {
        if ticket < active_switch.applied() {
            return Ok(());
        }
        store.set_active_game_dir(&target, &entries, found)?;
        active_switch.commit(ticket);
        Ok(())
    })?;
    state_for(config)
}

#[tauri::command]
pub async fn get_config(state: State<'_, AppState>) -> Result<Config, AppError> {
    state.note_first_ipc();
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.config())
        .await
        .map_err(join_error)
}

#[tauri::command]
pub async fn set_theme(state: State<'_, AppState>, theme: Theme) -> Result<Config, AppError> {
    state.note_first_ipc();
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state
            .update(|store| store.set_theme(theme))
            .map(|(_, config)| config)
    })
    .await
    .map_err(join_error)?
}

#[tauri::command]
pub fn get_startup_warning(state: State<'_, AppState>) -> Result<Option<String>, AppError> {
    state.note_first_ipc();
    Ok(state.startup_warning.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Arc};
    use std::time::Duration;

    #[test]
    fn snapshot_completes_while_writer_is_busy() {
        let root = tempfile::tempdir().unwrap();
        let config = Config {
            active_game_dir: Some(root.path().to_path_buf()),
            ..Config::default()
        };
        let state = Arc::new(AppState::new(
            ConfigStore::new(root.path().join("config.json"), config),
            None,
            root.path().to_path_buf(),
        ));
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let writer_state = Arc::clone(&state);
        let writer = std::thread::spawn(move || {
            writer_state
                .update(|store| {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    store.set_theme(Theme::Dark)
                })
                .unwrap();
        });
        started_rx.recv().unwrap();
        let (tx, rx) = mpsc::channel();
        let reader = Arc::clone(&state);
        let thread =
            std::thread::spawn(move || tx.send(reader.snapshot().unwrap().game_dir).unwrap());
        let result = rx.recv_timeout(Duration::from_millis(500));
        release_tx.send(()).unwrap();
        writer.join().unwrap();
        thread.join().unwrap();
        assert_eq!(result.unwrap(), root.path());
        assert_eq!(state.config().theme, Theme::Dark);
    }
    #[test]
    fn concurrent_adds_persist_both_directories() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.json");
        let state = AppState::new(
            ConfigStore::new(path.clone(), Config::default()),
            None,
            root.path().to_path_buf(),
        );
        let dirs: Vec<_> = ["first", "second"]
            .into_iter()
            .map(|name| {
                let path = root.path().join(name);
                std::fs::create_dir_all(path.join("saves")).unwrap();
                validate_game_dir(&path).unwrap()
            })
            .collect();
        let threads: Vec<_> = dirs
            .iter()
            .cloned()
            .map(|dir| {
                let state = state.clone();
                std::thread::spawn(move || state.update(|store| store.add_game_dir(dir)).unwrap())
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let persisted = mine_utils_core::config::load_config(&path).unwrap();
        for dir in dirs {
            assert!(persisted.game_dirs.contains(&dir));
        }
        assert_eq!(persisted.game_dirs.len(), 2);
        assert_eq!(state.config(), persisted);
    }

    #[test]
    fn older_switch_finishes_before_newer_switch() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("first");
        let second = root.path().join("second");
        for path in [&first, &second] {
            std::fs::create_dir_all(path.join("saves")).unwrap();
        }
        let mut store = ConfigStore::new(root.path().join("config.json"), Config::default());
        for path in [&first, &second] {
            store
                .add_game_dir(validate_game_dir(path).unwrap())
                .unwrap();
        }
        let state = AppState::new(store, None, root.path().to_path_buf());
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let older_state = state.clone();
        let older_path = first.to_string_lossy().into_owned();
        let older = std::thread::spawn(move || {
            set_active_with(&older_state, &older_path, |path| {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(std::fs::canonicalize(path).unwrap())
            })
            .unwrap();
        });
        entered_rx.recv().unwrap();
        let validation_holds_lock = state.store.try_lock().is_err();
        if !validation_holds_lock {
            let newer_path = second.to_string_lossy().into_owned();
            set_active_with(&state, &newer_path, |path| {
                Ok(std::fs::canonicalize(path).unwrap())
            })
            .unwrap();
            assert_eq!(state.config().active_game_dir, Some(second.clone()));
        }
        release_tx.send(()).unwrap();
        older.join().unwrap();
        assert!(
            !validation_holds_lock,
            "blocked validation holds the store lock"
        );
        assert_eq!(state.config().active_game_dir, Some(second));
    }

    #[test]
    fn blocked_validation_does_not_block_theme_write() {
        let root = tempfile::tempdir().unwrap();
        let game = root.path().join("game");
        std::fs::create_dir_all(game.join("saves")).unwrap();
        let mut store = ConfigStore::new(root.path().join("config.json"), Config::default());
        store
            .add_game_dir(validate_game_dir(&game).unwrap())
            .unwrap();
        let state = AppState::new(store, None, root.path().to_path_buf());
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let path = game.to_string_lossy().into_owned();
        let worker_state = state.clone();
        let worker = std::thread::spawn(move || {
            set_active_with(&worker_state, &path, |dir| {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(std::fs::canonicalize(dir).unwrap())
            })
            .unwrap();
        });
        entered_rx.recv().unwrap();
        let validation_holds_lock = state.store.try_lock().is_err();
        if !validation_holds_lock {
            state.update(|store| store.set_theme(Theme::Dark)).unwrap();
            assert_eq!(state.config().theme, Theme::Dark);
        }
        release_tx.send(()).unwrap();
        worker.join().unwrap();
        assert!(
            !validation_holds_lock,
            "blocked validation holds the store lock"
        );
        assert_eq!(state.config().theme, Theme::Dark);
        assert_eq!(
            state.config().active_game_dir.as_deref(),
            Some(game.as_path())
        );
    }

    #[test]
    fn failed_save_keeps_snapshot_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let blocker = root.path().join("file");
        std::fs::write(&blocker, b"x").unwrap();
        let state = AppState::new(
            ConfigStore::new(blocker.join("config.json"), Config::default()),
            None,
            root.path().to_path_buf(),
        );
        assert!(state.update(|store| store.set_theme(Theme::Dark)).is_err());
        assert_eq!(state.config().theme, Theme::System);
    }
}

#[cfg(test)]
mod ipc_tests {
    use super::*;
    use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, INVOKE_KEY};

    #[cfg(unix)]
    #[test]
    fn lists_saved_prism_dotminecraft_as_active() {
        use serde_json::{json, Value};

        const FIXTURE_HOME: &str = "MINE_UTILS_PRISM_FIXTURE_HOME";
        const INSTANCE: &str = ".local/share/PrismLauncher/instances/both";
        let Some(home) = std::env::var_os(FIXTURE_HOME).map(PathBuf::from) else {
            let root = tempfile::tempdir().unwrap();
            let home = root.path().canonicalize().unwrap();
            let instance = home.join(INSTANCE);
            for folder in ["minecraft", ".minecraft"] {
                std::fs::create_dir_all(instance.join(folder).join("saves")).unwrap();
            }
            std::fs::write(
                home.join("config.json"),
                json!({ "active_game_dir": instance.join(".minecraft") }).to_string(),
            )
            .unwrap();

            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "commands::game_dirs::ipc_tests::lists_saved_prism_dotminecraft_as_active",
                ])
                .env("HOME", &home)
                .env(FIXTURE_HOME, &home)
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&child.stdout);
            assert!(
                child.status.success() && stdout.contains("1 passed"),
                "Prism fixture subprocess failed:\n{stdout}{}",
                String::from_utf8_lossy(&child.stderr)
            );
            return;
        };

        assert_eq!(home_dir().unwrap(), home);
        let config_path = home.join("config.json");
        let config = mine_utils_core::load_config(&config_path).unwrap();
        let app = mock_builder()
            .manage(AppState::new(
                ConfigStore::new(config_path, config),
                None,
                home.clone(),
            ))
            .invoke_handler(tauri::generate_handler![get_config, list_game_dirs])
            .build(mock_context(noop_assets()))
            .unwrap();
        let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        let invoke = |cmd: &str| {
            get_ipc_response(
                &window,
                tauri::webview::InvokeRequest {
                    cmd: cmd.into(),
                    callback: tauri::ipc::CallbackFn(0),
                    error: tauri::ipc::CallbackFn(1),
                    url: "tauri://localhost".parse().unwrap(),
                    body: Default::default(),
                    headers: Default::default(),
                    invoke_key: INVOKE_KEY.into(),
                },
            )
            .unwrap()
            .deserialize::<Value>()
            .unwrap()
        };
        let instance = home.join(INSTANCE);
        let legacy = instance.join(".minecraft");
        let modern = json!({"path": instance.join("minecraft"), "source": "prism_launcher", "present": true, "stored": false});
        assert_eq!(invoke("get_config")["active_game_dir"], json!(legacy));
        assert_eq!(
            invoke("list_game_dirs"),
            json!([
                modern,
                {"path": legacy, "source": "manual", "present": true, "stored": false}
            ])
        );

        std::fs::remove_dir(legacy.join("saves")).unwrap();
        assert_eq!(invoke("list_game_dirs"), json!([modern]));
    }

    #[test]
    fn command_persists_world_to_disk() {
        let root = tempfile::tempdir().unwrap();
        let game = root.path().join("game");
        let world = game.join("saves/creative");
        std::fs::create_dir_all(&world).unwrap();
        std::fs::write(world.join("level.dat"), b"level").unwrap();
        let path = root.path().join("config.json");
        let mut config = Config::default();
        config.set_active_game_dir(&game);
        let app = mock_builder()
            .manage(AppState::new(
                ConfigStore::new(path.clone(), config),
                None,
                root.path().to_path_buf(),
            ))
            .invoke_handler(tauri::generate_handler![set_last_world])
            .build(mock_context(noop_assets()))
            .unwrap();
        let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        let result = get_ipc_response(
            &window,
            tauri::webview::InvokeRequest {
                cmd: "set_last_world".into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: "tauri://localhost".parse().unwrap(),
                body: tauri::ipc::InvokeBody::Json(serde_json::json!({"path": world})),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.into(),
            },
        );
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(
            mine_utils_core::load_config(&path).unwrap().last_world,
            Some(world)
        );
    }
}
