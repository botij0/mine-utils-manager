mod commands;

use mine_utils_core::{load_config, AppPaths, Config};
use tauri::Manager;

use commands::game_dirs::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .target(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::Stdout,
                ))
                .build(),
        )
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .setup(|app| {
            let paths = AppPaths::resolve()?;
            let config = match load_config(&paths.config_file()) {
                Ok(config) => config,
                Err(err) => {
                    log::warn!("starting with a fresh config: {err}");
                    Config::default()
                }
            };
            app.manage(AppState::new(paths, config));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::game_dirs::list_game_dirs,
            commands::game_dirs::add_game_dir,
            commands::game_dirs::remove_game_dir,
            commands::game_dirs::set_active_game_dir,
            commands::game_dirs::get_config,
            commands::game_dirs::save_config,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
