use serde::Serialize;

macro_rules! error_codes {
    ($($variant:ident => $code:literal),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
        #[cfg_attr(feature = "wire-types", derive(ts_rs::TS))]
        pub enum ErrorCode {
            $(#[serde(rename = $code)] $variant),*
        }

        impl ErrorCode {
            #[cfg(feature = "wire-types")]
            pub const ALL: &'static [Self] = &[$(Self::$variant),*];

            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $code),* }
            }
        }
    };
}

error_codes! {
    Backup => "backup",
    Config => "config",
    DownloadCancelled => "download_cancelled",
    DownloadExists => "download_exists",
    DownloadFilename => "download_filename",
    DownloadHashMismatch => "download_hash_mismatch",
    DownloadHost => "download_host",
    DownloadIo => "download_io",
    DownloadNotZip => "download_not_zip",
    DownloadOffline => "download_offline",
    DownloadStatus => "download_status",
    DownloadTimeout => "download_timeout",
    DownloadTooLarge => "download_too_large",
    DownloadWrongRoot => "download_wrong_root",
    GameRunning => "game_running",
    InvalidGameDir => "invalid_game_dir",
    Io => "io",
    LevelBackup => "level_backup",
    LevelDat => "level_dat",
    ModrinthBody => "modrinth_body",
    ModrinthOffline => "modrinth_offline",
    ModrinthParse => "modrinth_parse",
    ModrinthRedirect => "modrinth_redirect",
    ModrinthStatus => "modrinth_status",
    ModrinthUrl => "modrinth_url",
    NoActiveGameDir => "no_active_game_dir",
    OptionsTxt => "options_txt",
    PackBrokenArchive => "pack_broken_archive",
    PackDuplicate => "pack_duplicate",
    PackMissingMeta => "pack_missing_meta",
    PackNameCollision => "pack_name_collision",
    PackNonUtf8Name => "pack_non_utf8_name",
    PackNotFound => "pack_not_found",
    PackOutsideRoot => "pack_outside_root",
    PackSymlink => "pack_symlink",
    PackTrash => "pack_trash",
    PackUnknownKind => "pack_unknown_kind",
    ProvenanceCorrupt => "provenance_corrupt",
    ProvenanceIo => "provenance_io",
    ResourceStack => "resource_stack",
    StaleStack => "stale_stack",
    UnknownGameDir => "unknown_game_dir",
    World => "world",
    WorldMismatch => "world_mismatch",
}
