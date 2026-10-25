pub mod backup;
pub mod config;
pub mod error;
pub mod format_table;
pub mod game_dir;
pub mod game_lock;
pub mod icon_cache;
pub mod options_txt;
pub mod pack;
pub mod pack_meta;
pub mod pack_scan;
pub mod paths;
pub mod resource_stack;

pub use backup::{BackupInfo, BackupOutcome};
pub use config::{
    load_config, save_config, BackupsConfig, Config, ConfigStore, Theme, CONFIG_SCHEMA,
};
pub use error::AppError;
pub use format_table::{FormatRangeEntry, FormatTable};
pub use game_dir::{
    contains_entry, discover_game_dirs, entry_for, is_game_dir, merge_game_dirs, validate_game_dir,
    DiscoverySource, GameDir, GameDirEntry,
};
pub use game_lock::{GameRunning, GameRunningKind, GameStatus};
pub use icon_cache::{icon_within_pixel_cap, png_dimensions, IconCache, MAX_ICON_DIMENSION};
pub use options_txt::{read_options, write_options, OptionsError, OptionsTxt};
pub use pack::{
    FormatRange, FormatVersion, ImportChoice, ImportOutcome, Pack, PackError, PackIcon, PackIssue,
    PackKind, PackLocation, PackMeta,
};
pub use pack_scan::{
    delete_pack, delete_pack_with, import_pack, import_pack_with, pack_names, scan_packs,
    HashCache, PackListing, SystemTrash, TrashService, MAX_ICON_BYTES, MAX_META_BYTES,
    MAX_ZIP_ENTRIES,
};
pub use paths::{home_dir, AppPaths};
pub use resource_stack::{
    ResourceStack, StackEntry, StackError, StackPaths, StackSnapshot, StackWrite,
};
