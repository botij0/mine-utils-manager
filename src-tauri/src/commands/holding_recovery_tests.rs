use mine_utils_core::{Config, ConfigStore};
use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, INVOKE_KEY};

use super::{game_dirs::AppState, resource_stack::StackState};

#[test]
fn holding_commands_restore_files_and_return_collision_errors() {
    for (command, relative) in [
        ("restore_resource_holding", "resourcepacks"),
        ("restore_datapack_holding", "saves/World/datapacks"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let game = root.path().join("game");
        let folder = game.join(relative);
        std::fs::create_dir_all(&folder).unwrap();
        let holding = ".mine-utils-replace-Old.zip.4294967295-123-0.tmp";
        std::fs::write(folder.join(holding), b"old pack").unwrap();
        let mut config = Config::default();
        config.set_active_game_dir(&game);
        let app = mock_builder()
            .manage(AppState::new(
                ConfigStore::new(root.path().join("config.json"), config),
                None,
                root.path().join("data"),
            ))
            .manage(StackState::default())
            .invoke_handler(tauri::generate_handler![
                super::packs::restore_resource_holding,
                super::worlds::restore_datapack_holding
            ])
            .build(mock_context(noop_assets()))
            .unwrap();
        let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        let invoke = || {
            get_ipc_response(
                &window,
                tauri::webview::InvokeRequest {
                    cmd: command.into(),
                    callback: tauri::ipc::CallbackFn(0),
                    error: tauri::ipc::CallbackFn(1),
                    url: "tauri://localhost".parse().unwrap(),
                    body: tauri::ipc::InvokeBody::Json(
                        serde_json::json!({"holdingName": holding, "world": "World"}),
                    ),
                    headers: Default::default(),
                    invoke_key: INVOKE_KEY.into(),
                },
            )
        };
        assert!(invoke().is_ok());
        assert_eq!(std::fs::read(folder.join("Old.zip")).unwrap(), b"old pack");
        std::fs::write(folder.join(holding), b"leftover").unwrap();
        let err = invoke().unwrap_err();
        assert_eq!(err["code"], "io");
        assert!(err["message"]
            .as_str()
            .unwrap()
            .contains("original name must be free"));
        assert_eq!(std::fs::read(folder.join(holding)).unwrap(), b"leftover");
        assert_eq!(std::fs::read(folder.join("Old.zip")).unwrap(), b"old pack");
    }
}
