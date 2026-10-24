pub mod config;
pub mod error;
pub mod format_table;
pub mod game_dir;
pub mod icon_cache;
pub mod pack;
pub mod pack_meta;
pub mod pack_scan;
pub mod paths;

pub use config::{
    load_config, save_config, BackupsConfig, Config, ConfigStore, Theme, CONFIG_SCHEMA,
};
pub use error::AppError;
pub use format_table::{FormatRangeEntry, FormatTable};
pub use game_dir::{
    contains_entry, discover_game_dirs, entry_for, is_game_dir, merge_game_dirs, validate_game_dir,
    DiscoverySource, GameDir, GameDirEntry,
};
pub use icon_cache::IconCache;
pub use pack::{
    FormatRange, FormatVersion, ImportChoice, ImportOutcome, Pack, PackError, PackIcon, PackIssue,
    PackKind, PackLocation, PackMeta,
};
pub use pack_scan::{delete_pack, import_pack, scan_packs, PackListing};
pub use paths::{home_dir, AppPaths};
