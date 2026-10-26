mod commands;

use mine_utils_core::{load_config, AppPaths, Config, ConfigStore};
use tauri::Manager;

use commands::game_dirs::AppState;
use commands::packs::PackState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
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
        .setup(|app| {
            let paths = AppPaths::resolve()?;
            let config_path = paths.config_file();
            let (config, startup_warning) = match load_config(&config_path) {
                Ok(config) => (config, None),
                Err(err) => {
                    let stamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|duration| duration.as_secs())
                        .unwrap_or(0);
                    let backup = config_path.with_extension(format!("json.corrupt-{stamp}"));
                    let backed_up = std::fs::rename(&config_path, &backup)
                        .or_else(|_| std::fs::copy(&config_path, &backup).map(|_| ()));
                    let warning = match backed_up {
                        Ok(()) => format!(
                            "The config file could not be read. A backup is at {}. The app started with defaults.",
                            backup.display()
                        ),
                        Err(backup_err) => format!(
                            "The config file could not be read and could not be backed up ({}). The app started with defaults.",
                            backup_err
                        ),
                    };
                    log::warn!("{warning} ({err})");
                    (Config::default(), Some(warning))
                }
            };
            app.manage(AppState::new(
                ConfigStore::new(config_path, config),
                startup_warning,
                paths.data_dir.clone(),
            ));
            app.manage(PackState::new(&paths.cache_dir));
            app.manage(commands::resource_stack::StackState::default());
            app.manage(commands::discover::ModrinthState::new());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::game_dirs::list_game_dirs,
            commands::game_dirs::add_game_dir,
            commands::game_dirs::remove_game_dir,
            commands::game_dirs::set_active_game_dir,
            commands::game_dirs::get_config,
            commands::game_dirs::set_theme,
            commands::game_dirs::get_startup_warning,
            commands::packs::list_resource_packs,
            commands::packs::import_resource_pack,
            commands::packs::delete_resource_pack,
            commands::resource_stack::get_resource_stack,
            commands::resource_stack::set_resource_pack_enabled,
            commands::resource_stack::reorder_resource_stack,
            commands::resource_stack::list_options_backups,
            commands::resource_stack::restore_options_backup,
            commands::worlds::list_worlds,
            commands::worlds::list_datapacks,
            commands::worlds::import_datapack,
            commands::worlds::set_datapack_enabled,
            commands::worlds::reorder_datapacks,
            commands::worlds::delete_datapack,
            commands::worlds::remove_datapack_reference,
            commands::worlds::list_level_backups,
            commands::worlds::list_all_level_backups,
            commands::worlds::restore_level_backup,
            commands::discover::search_modrinth,
            commands::discover::get_modrinth_project,
            commands::discover::list_modrinth_versions,
            commands::discover::modrinth_targets,
            commands::discover::install_modrinth_pack,
            commands::discover::get_modrinth_provenance,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
