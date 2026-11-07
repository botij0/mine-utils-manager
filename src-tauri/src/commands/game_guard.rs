use std::path::PathBuf;

use mine_utils_core::{game_lock, AppError, GameRunning, GameStatus};
use serde::Serialize;
use tauri::State;

use super::game_dirs::AppState;
use super::join_error;

/// The game-running guard on its own, in the same wire shape the resource and
/// datapack stacks carry. It never reads `options.txt`, so a missing or
/// malformed file cannot hide whether the game is running.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GameGuard {
    pub running: Option<GameRunning>,
    /// Stale `session.lock` files. They never block a write.
    pub stale_locks: Vec<PathBuf>,
    /// False where POSIX lock probing is unsupported.
    pub detection_supported: bool,
}

impl From<GameStatus> for GameGuard {
    fn from(status: GameStatus) -> Self {
        GameGuard {
            running: status.running,
            stale_locks: status.stale_locks,
            detection_supported: status.supported,
        }
    }
}

async fn game_guard(state: &AppState) -> Result<GameGuard, AppError> {
    let game_dir = state.active_game_dir().ok_or(AppError::NoActiveGameDir)?;
    tauri::async_runtime::spawn_blocking(move || GameGuard::from(game_lock::check(&game_dir)))
        .await
        .map_err(join_error)
}

/// Check the active game dir for a running client. The UI polls this.
#[tauri::command]
pub async fn get_game_guard(state: State<'_, AppState>) -> Result<GameGuard, AppError> {
    state.note_ipc();
    game_guard(&state).await
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use mine_utils_core::{resource_stack, Config, ConfigStore};

    use super::*;

    fn state_for(root: &Path, active_game_dir: Option<PathBuf>) -> AppState {
        let config = Config {
            active_game_dir,
            ..Config::default()
        };
        AppState::new(
            ConfigStore::new(root.join("config.json"), config),
            None,
            root.join("data"),
        )
    }

    #[test]
    fn reports_the_guard_for_a_game_dir_without_options_txt() {
        let root = tempfile::tempdir().expect("tempdir");
        let game_dir = root.path().join("minecraft");
        let world = game_dir.join("saves").join("world");
        std::fs::create_dir_all(&world).expect("world dir");
        std::fs::write(world.join("session.lock"), b"").expect("stale lock");
        let state = state_for(root.path(), Some(game_dir.clone()));

        let stack_err = resource_stack::load(&state.stack_paths().expect("paths"))
            .expect_err("the stack read needs options.txt");
        assert_eq!(stack_err.code(), "options_txt");

        let guard = tauri::async_runtime::block_on(game_guard(&state)).expect("guard");

        assert_eq!(
            serde_json::to_value(&guard).expect("serializes"),
            serde_json::json!({
                "running": null,
                "stale_locks": [world.join("session.lock")],
                "detection_supported": true,
            })
        );
        assert!(!game_dir.join("options.txt").exists());
    }

    #[test]
    fn reports_no_active_game_dir_without_a_folder() {
        let root = tempfile::tempdir().expect("tempdir");
        let state = state_for(root.path(), None);

        let err = tauri::async_runtime::block_on(game_guard(&state)).expect_err("no folder");

        assert_eq!(err.code(), "no_active_game_dir");
    }
}
