pub mod backup;
pub mod config;
pub mod data_stack;
pub mod download;
pub mod error;
pub mod format_table;
mod fs_commit;
pub mod game_dir;
pub mod game_lock;
pub mod icon_cache;
pub mod install;
pub mod level_backup;
pub mod level_dat;
pub mod modrinth;
pub mod options_txt;
pub mod pack;
pub mod pack_meta;
pub mod pack_scan;
pub mod paths;
pub mod provenance;
pub mod resource_stack;
pub mod world;

pub use backup::{BackupInfo, BackupOutcome};
pub use config::{
    load_config, save_config, BackupsConfig, Config, ConfigStore, Theme, CONFIG_SCHEMA,
};
pub use data_stack::{
    DataPaths, DataSnapshot, DataStack, DataStackEntry, DataStackError, DataWrite,
    DatapackDeleteOutcome, DatapackImportOutcome,
};
pub use download::{
    commit_verified, download_verified, download_verified_capped, validate_filename, DownloadError,
    VerifiedDownload, ZipRoot,
};
pub use error::AppError;
pub use format_table::{
    data_format_for_version, newest_known_version_if_newer, FormatRangeEntry, FormatTable,
};
pub use game_dir::{
    contains_entry, discover_game_dirs, entry_for, is_game_dir, merge_game_dirs, validate_game_dir,
    DiscoverySource, GameDir, GameDirEntry,
};
pub use game_lock::{GameRunning, GameRunningKind, GameStatus};
pub use icon_cache::{icon_within_pixel_cap, png_dimensions, IconCache, MAX_ICON_DIMENSION};
pub use install::{
    finish_modrinth_install, parse_install_kind, prepare_modrinth_install, CoreInstallOutcome,
    PreparedInstall, PreparedSource,
};
pub use level_backup::{LevelBackupInfo, LevelBackupOutcome};
pub use level_dat::{LevelDatError, LevelDatView};
pub use modrinth::{
    installed_game_versions, is_allowed_icon_url, prefer_version, preferred_launcher_version,
    primary_file, sort_game_versions, validate_modrinth_id, ModrinthClient, ModrinthError,
    ModrinthFile, ModrinthHashes, ModrinthProject, ModrinthVersion, RawResponse, ReqwestTransport,
    SearchHit, SearchKind, SearchQuery, SearchSort, Transport, ALLOWED_HOSTS, API_BASE, CDN_HOST,
    DOWNLOAD_CONNECT_TIMEOUT_SECS, DOWNLOAD_TIMEOUT_SECS, MAX_API_BODY, MAX_DOWNLOAD_BYTES,
    MAX_REDIRECTS, MAX_RETRIES, REQUEST_TIMEOUT_SECS,
};
pub use options_txt::{read_options, write_options, OptionsError, OptionsTxt};
pub use pack::{
    FormatRange, FormatVersion, ImportChoice, ImportOutcome, Pack, PackError, PackIcon, PackIssue,
    PackKind, PackLocation, PackMeta,
};
pub use pack_scan::{
    delete_pack, delete_pack_with, import_pack, import_pack_with, pack_names, scan_packs,
    validate_delete_target, HashCache, PackListing, SystemTrash, TrashService, MAX_ICON_BYTES,
    MAX_META_BYTES, MAX_ZIP_ENTRIES,
};
pub use paths::{home_dir, AppPaths};
pub use provenance::{
    lookup as provenance_lookup, provenance_file, record as provenance_record, ProvenanceEntry,
    ProvenanceError,
};
pub use resource_stack::{
    DeleteOutcome, ResourceStack, StackEntry, StackError, StackPaths, StackSnapshot, StackWrite,
};
pub use world::{World, WorldIssue, WorldPaths, WorldVersion};
