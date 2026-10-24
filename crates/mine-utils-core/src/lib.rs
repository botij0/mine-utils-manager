pub mod config;
pub mod error;
pub mod game_dir;
pub mod paths;

pub use config::{load_config, save_config, BackupsConfig, Config, Theme, CONFIG_SCHEMA};
pub use error::AppError;
pub use game_dir::{
    contains_entry, discover_game_dirs, entry_for, is_game_dir, merge_game_dirs, validate_game_dir,
    DiscoverySource, GameDir, GameDirEntry,
};
pub use paths::{home_dir, AppPaths};
