//! Scan the `saves` directory: one entry per world with its name, version,
//! icon, lock state, and any error that makes it read-only.
//!
//! Only direct children of `saves` are scanned, and a directory without
//! `level.dat` is skipped, never an error. A corrupt `level.dat`, a missing
//! version, or malformed datapack lists render as an error card; the other
//! worlds still render.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::format_table::{data_format_for_version, newest_known_version_if_newer};
use crate::game_lock::{self, GameRunning};
use crate::icon_cache::IconCache;
use crate::level_dat;
use crate::pack::FormatVersion;
use crate::pack::PackIcon;
use crate::pack_scan::MAX_ICON_BYTES;
use crate::read_capped::read_capped;

/// A world's game version: the `Data.Version.Name` string plus the data pack
/// format the world's datapacks must declare, when the table covers it.
/// `newest_known_version` is set only when the world is newer than every
/// version the table covers, so a missing `data_format` can be told apart
/// from a game that is newer than this build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "wire-types", derive(ts_rs::TS))]
pub struct WorldVersion {
    pub name: String,
    pub data_format: Option<FormatVersion>,
    pub newest_known_version: Option<String>,
}

/// Why a world is read-only. The write controls hide while this is set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "wire-types", derive(ts_rs::TS))]
pub struct WorldIssue {
    pub code: String,
    pub message: String,
}

/// One world as the UI lists it. `issue` is set for corrupt, version-less, or
/// worlds with malformed metadata; those render an error card instead of write controls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "wire-types", derive(ts_rs::TS))]
pub struct World {
    pub dir_name: String,
    pub path: PathBuf,
    pub level_name: String,
    pub version: Option<WorldVersion>,
    pub icon: Option<PackIcon>,
    pub running: Option<GameRunning>,
    pub world_locked: Option<GameRunning>,
    pub stale_lock: bool,
    pub detection_supported: bool,
    pub issue: Option<WorldIssue>,
}

/// Where one world lives on disk. The name must be a direct child of `saves`,
/// so a crafted name can never escape the game dir.
#[derive(Debug, Clone)]
pub struct WorldPaths {
    game_dir: PathBuf,
    world_dir: PathBuf,
    name: String,
}

impl WorldPaths {
    pub fn game_dir(&self) -> &Path {
        &self.game_dir
    }
    pub fn world_dir(&self) -> &Path {
        &self.world_dir
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn level_dat_path(&self) -> PathBuf {
        level_dat_in(&self.world_dir)
    }

    pub fn for_world(game_dir: &Path, world: &str) -> Result<WorldPaths, crate::error::AppError> {
        Self::for_world_with(game_dir, world, Path::canonicalize)
    }

    fn for_world_with(
        game_dir: &Path,
        world: &str,
        canonicalize_world: impl FnOnce(&Path) -> std::io::Result<PathBuf>,
    ) -> Result<WorldPaths, crate::error::AppError> {
        if world.is_empty()
            || world == "."
            || world == ".."
            || world.contains('/')
            || world.contains('\\')
            || world.contains('\0')
        {
            return Err(crate::error::AppError::World {
                reason: format!("{world} is not a world in this game dir"),
            });
        }
        let world_dir = game_dir.join("saves").join(world);
        let resolved = resolve_inside_saves(game_dir, &world_dir, canonicalize_world)?;
        Ok(WorldPaths {
            game_dir: game_dir.to_path_buf(),
            world_dir: resolved,
            name: world.to_string(),
        })
    }
}

/// Resolve a world dir and prove it stays inside the canonical `saves` root.
/// A symlinked `saves/<name>` (even one pointing elsewhere on disk) is
/// rejected before any write, import, or delete touches it.
fn resolve_inside_saves(
    game_dir: &Path,
    world_dir: &Path,
    canonicalize_world: impl FnOnce(&Path) -> std::io::Result<PathBuf>,
) -> Result<PathBuf, crate::error::AppError> {
    let reject = |reason: String| {
        Err(crate::error::AppError::World {
            reason: format!("{}: {reason}", world_dir.display()),
        })
    };
    if let Ok(meta) = std::fs::symlink_metadata(world_dir) {
        if meta.file_type().is_symlink() {
            return reject("a symlinked world is never a write target".to_string());
        }
    }
    let canonical_game = std::fs::canonicalize(game_dir).unwrap_or_else(|_| game_dir.to_path_buf());
    let canonical_saves = std::fs::canonicalize(canonical_game.join("saves"))
        .unwrap_or_else(|_| canonical_game.join("saves"));
    let resolved = canonicalize_world(world_dir)
        .unwrap_or_else(|_| canonical_saves.join(world_dir.file_name().unwrap_or_default()));
    if resolved != canonical_saves && !resolved.starts_with(&canonical_saves) {
        return reject(format!("is outside {}", canonical_saves.display()));
    }
    Ok(resolved)
}

/// True only for a real directory: symlinks (even to directories) are
/// rejected so a linked `saves` entry can never become a write target.
fn is_real_dir(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => meta.file_type().is_dir() && !meta.file_type().is_symlink(),
        Err(_) => false,
    }
}

/// A real `level.dat` file. A symlink is never followed: it could point
/// outside the game dir.
pub(crate) fn is_level_file(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => meta.file_type().is_file() && !meta.file_type().is_symlink(),
        Err(_) => false,
    }
}

fn issue(err: &level_dat::LevelDatError) -> WorldIssue {
    WorldIssue {
        code: err.code().to_string(),
        message: err.to_string(),
    }
}

/// List every world in `saves`, sorted by directory name. A missing `saves`
/// directory means no worlds, not an error. Symlinked entries are skipped:
/// following one could read or write outside the game dir.
/// The one place a world's `level.dat` path is derived.
pub(crate) fn level_dat_in(world_dir: &Path) -> PathBuf {
    world_dir.join("level.dat")
}

pub fn scan_worlds(game_dir: &Path, icons: &IconCache) -> Vec<World> {
    let saves = game_dir.join("saves");
    let mut dirs: Vec<PathBuf> = match std::fs::read_dir(&saves) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| is_real_dir(path))
            .collect(),
        Err(_) => return Vec::new(),
    };
    dirs.sort();
    // One game-guard probe for the whole scan; per-world session locks are
    // still probed individually inside `read_world`.
    let status = game_lock::check(game_dir);
    dirs.into_iter()
        .filter_map(|dir| read_world(&dir, icons, &status))
        .collect()
}

fn read_world(dir: &Path, icons: &IconCache, status: &game_lock::GameStatus) -> Option<World> {
    let dir_name = dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_string();
    if dir_name.is_empty() {
        return None;
    }
    if !is_real_dir(dir) {
        // A symlinked `saves` entry is never a world the app opens.
        return None;
    }
    if !is_level_file(&level_dat_in(dir)) {
        // Not a world the app can open; skip it silently.
        return None;
    }
    let world_locked = game_lock::world_locked(dir);
    let stale_lock = dir.join("session.lock").is_file()
        && world_locked.is_none()
        && game_lock::lock_detection_supported();
    let mut world = World {
        dir_name: dir_name.clone(),
        path: dir.to_path_buf(),
        level_name: dir_name.clone(),
        version: None,
        icon: None,
        running: status.running.clone(),
        world_locked,
        stale_lock,
        detection_supported: status.supported,
        issue: None,
    };
    match read_capped(&level_dat_in(dir), level_dat::MAX_LEVEL_DAT_BYTES) {
        Ok(bytes) => match level_dat::parse_lenient(&bytes) {
            Ok(view) => {
                if let Some(name) = view.level_name {
                    if !name.is_empty() {
                        world.level_name = name;
                    }
                }
                world.version = Some(WorldVersion {
                    data_format: data_format_for_version(&view.version_name),
                    newest_known_version: newest_known_version_if_newer(&view.version_name),
                    name: view.version_name,
                });
            }
            Err(err) => {
                world.issue = Some(issue(&err));
            }
        },
        Err(err) => {
            world.issue = Some(WorldIssue {
                code: "level_dat".to_string(),
                message: format!("could not read level.dat: {err}"),
            });
        }
    }
    if let Ok(bytes) = read_capped(&dir.join("icon.png"), MAX_ICON_BYTES) {
        world.icon = icons.store(&bytes).ok();
    }
    Some(world)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level_dat_bytes(version: Option<&str>, with_packs: bool) -> Vec<u8> {
        let mut data: std::collections::HashMap<String, fastnbt::Value> =
            std::collections::HashMap::new();
        if let Some(version) = version {
            data.insert(
                "Version".to_string(),
                fastnbt::Value::Compound(
                    [(
                        "Name".to_string(),
                        fastnbt::Value::String(version.to_string()),
                    )]
                    .into_iter()
                    .collect(),
                ),
            );
        }
        if with_packs {
            data.insert(
                "DataPacks".to_string(),
                fastnbt::Value::Compound(
                    [
                        ("Enabled".to_string(), fastnbt::Value::List(vec![])),
                        ("Disabled".to_string(), fastnbt::Value::List(vec![])),
                    ]
                    .into_iter()
                    .collect(),
                ),
            );
        }
        data.insert(
            "LevelName".to_string(),
            fastnbt::Value::String("Scanned".to_string()),
        );
        let root = fastnbt::Value::Compound(
            [("Data".to_string(), fastnbt::Value::Compound(data))]
                .into_iter()
                .collect(),
        );
        level_dat::gzip(&fastnbt::to_bytes(&root).unwrap())
    }

    fn game_with(worlds: &[(&str, Option<Vec<u8>>)]) -> (tempfile::TempDir, IconCache) {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        for (name, level) in worlds {
            let world = game.join("saves").join(name);
            std::fs::create_dir_all(&world).unwrap();
            if let Some(bytes) = level {
                std::fs::write(world.join("level.dat"), bytes).unwrap();
            }
        }
        let icons = IconCache::new(&dir.path().join("cache"));
        (dir, icons)
    }

    #[test]
    fn oversized_and_symlinked_icons_are_ignored() {
        let good = level_dat_bytes(Some("1.21.10"), true);
        let (dir, icons) = game_with(&[("large", Some(good.clone())), ("linked", Some(good))]);
        let saves = dir.path().join("game/saves");
        std::fs::write(saves.join("large/icon.png"), vec![1; 3 * 1024 * 1024]).unwrap();
        #[cfg(unix)]
        {
            let target = dir.path().join("icon");
            std::fs::write(&target, b"icon").unwrap();
            std::os::unix::fs::symlink(target, saves.join("linked/icon.png")).unwrap();
        }
        let worlds = scan_worlds(&dir.path().join("game"), &icons);
        assert_eq!(worlds.len(), 2);
        for world in worlds {
            assert_eq!(world.icon, None);
            assert_eq!(world.issue, None);
            assert_eq!(world.version.unwrap().name, "1.21.10");
        }
    }

    #[test]
    fn oversized_level_data_reports_issue_and_keeps_other_worlds() {
        let bomb = level_dat::gzip(&vec![0; 64 * 1024 * 1024 + 1]);
        let (dir, icons) = game_with(&[
            ("bad", Some(bomb)),
            ("good", Some(level_dat_bytes(Some("1.21.10"), true))),
        ]);
        let worlds = scan_worlds(&dir.path().join("game"), &icons);
        assert_eq!(worlds.len(), 2);
        assert_eq!(worlds[0].issue.as_ref().unwrap().code, "level_dat");
        assert!(worlds[0]
            .issue
            .as_ref()
            .unwrap()
            .message
            .contains("too large"));
        assert_eq!(worlds[1].version.as_ref().unwrap().name, "1.21.10");
        assert_eq!(worlds[1].issue, None);
    }

    #[test]
    fn skips_directories_without_level_dat() {
        let (dir, icons) = game_with(&[
            ("real", Some(level_dat_bytes(Some("1.21.10"), true))),
            ("empty", None),
        ]);
        let worlds = scan_worlds(&dir.path().join("game"), &icons);
        assert_eq!(worlds.len(), 1);
        assert_eq!(worlds[0].dir_name, "real");
        assert_eq!(worlds[0].version.as_ref().unwrap().name, "1.21.10");
    }

    #[test]
    fn corrupt_level_dat_is_an_error_card() {
        let (dir, icons) = game_with(&[("bad", Some(b"not gzip".to_vec()))]);
        let worlds = scan_worlds(&dir.path().join("game"), &icons);
        assert_eq!(worlds.len(), 1);
        assert!(worlds[0].issue.is_some());
        assert!(worlds[0].version.is_none());
    }

    #[test]
    fn missing_version_is_an_error_card() {
        let (dir, icons) = game_with(&[("noversion", Some(level_dat_bytes(None, true)))]);
        let worlds = scan_worlds(&dir.path().join("game"), &icons);
        assert!(worlds[0].issue.is_some());
    }

    #[test]
    fn missing_datapacks_lists_normally() {
        let (dir, icons) = game_with(&[("nopacks", Some(level_dat_bytes(Some("1.12.2"), false)))]);
        let worlds = scan_worlds(&dir.path().join("game"), &icons);
        assert_eq!(worlds[0].issue, None);
        assert_eq!(worlds[0].version.as_ref().unwrap().name, "1.12.2");
    }

    #[test]
    fn malformed_datapacks_is_an_error_card() {
        let bytes = level_dat_bytes(Some("1.12.2"), false);
        let mut root = level_dat::parse_nbt(&level_dat::gunzip(&bytes).unwrap()).unwrap();
        if let fastnbt::Value::Compound(ref mut root) = root {
            if let Some(fastnbt::Value::Compound(data)) = root.get_mut("Data") {
                data.insert(
                    "DataPacks".to_string(),
                    fastnbt::Value::String("bad".to_string()),
                );
            }
        }
        let bytes = level_dat::gzip(&fastnbt::to_bytes(&root).unwrap());
        let (dir, icons) = game_with(&[("bad", Some(bytes))]);
        let worlds = scan_worlds(&dir.path().join("game"), &icons);
        assert_eq!(worlds[0].issue.as_ref().unwrap().code, "level_dat");
    }

    #[test]
    fn missing_icon_is_none_not_an_error() {
        let (dir, icons) = game_with(&[("noicon", Some(level_dat_bytes(Some("1.21.10"), true)))]);
        let worlds = scan_worlds(&dir.path().join("game"), &icons);
        assert!(worlds[0].icon.is_none());
        assert!(worlds[0].issue.is_none());
    }

    #[test]
    fn falls_back_to_the_directory_name() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        let world = game.join("saves").join("DirName");
        std::fs::create_dir_all(&world).unwrap();
        let mut data: std::collections::HashMap<String, fastnbt::Value> =
            std::collections::HashMap::new();
        data.insert(
            "Version".to_string(),
            fastnbt::Value::Compound(
                [(
                    "Name".to_string(),
                    fastnbt::Value::String("1.21.10".to_string()),
                )]
                .into_iter()
                .collect(),
            ),
        );
        data.insert(
            "DataPacks".to_string(),
            fastnbt::Value::Compound(
                [
                    ("Enabled".to_string(), fastnbt::Value::List(vec![])),
                    ("Disabled".to_string(), fastnbt::Value::List(vec![])),
                ]
                .into_iter()
                .collect(),
            ),
        );
        let root = fastnbt::Value::Compound(
            [("Data".to_string(), fastnbt::Value::Compound(data))]
                .into_iter()
                .collect(),
        );
        std::fs::write(
            world.join("level.dat"),
            level_dat::gzip(&fastnbt::to_bytes(&root).unwrap()),
        )
        .unwrap();
        let icons = IconCache::new(&dir.path().join("cache"));
        let worlds = scan_worlds(&game, &icons);
        assert_eq!(worlds[0].level_name, "DirName");
    }

    #[test]
    fn world_names_cannot_escape_saves() {
        let game = Path::new("/game");
        for bad in ["", ".", "..", "a/b", "a\\b", "a\0b"] {
            assert!(WorldPaths::for_world(game, bad).is_err());
        }
        assert!(WorldPaths::for_world(game, "Survival 1_17").is_ok());
    }

    #[test]
    fn case_alias_shares_level_backup_retention_and_listing() {
        use crate::{config::BackupsConfig, data_stack::DataPaths, level_backup};

        let (dir, _) = game_with(&[("world", None)]);
        let game = dir.path().join("game");
        let exact = WorldPaths::for_world(&game, "world").unwrap();
        let alias = WorldPaths::for_world_with(&game, "World", |path| {
            assert_eq!(path, game.join("saves/World"));
            path.canonicalize()
                .or_else(|_| path.with_file_name("world").canonicalize())
        })
        .unwrap();
        let config = BackupsConfig {
            enabled: true,
            keep: 1,
        };
        let exact = DataPaths::for_world(&exact, dir.path(), config.clone());
        let alias = DataPaths::for_world(&alias, dir.path(), config);

        for (paths, bytes) in [(&exact, &b"older"[..]), (&alias, &b"new backup"[..])] {
            level_backup::create(
                paths.backups_root(),
                paths.world_name(),
                bytes,
                paths.backups(),
            )
            .unwrap();
        }

        let backups = level_backup::list_all(exact.backups_root()).unwrap();
        assert_eq!(
            backups
                .iter()
                .map(|backup| backup.world.as_str())
                .collect::<Vec<_>>(),
            ["world"],
        );
        assert_eq!(
            std::fs::read(exact.backups_root().join(&backups[0].file_name)).unwrap(),
            b"new backup",
        );
        for paths in [exact, alias] {
            assert_eq!(
                level_backup::list(paths.backups_root(), paths.world_name()).unwrap(),
                backups,
            );
        }
    }

    #[test]
    fn level_backup_name_preserves_exact_case() {
        let (dir, _) = game_with(&[("My World", None)]);
        let world = WorldPaths::for_world(&dir.path().join("game"), "My World").unwrap();
        let paths = crate::data_stack::DataPaths::for_world(&world, dir.path(), Default::default());
        assert_eq!(paths.world_name(), "My World");
    }

    #[test]
    fn symlinked_worlds_are_skipped_and_rejected_for_writes() {
        let (dir, icons) = game_with(&[("real", Some(level_dat_bytes(Some("1.21.10"), true)))]);
        let game = dir.path().join("game");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("level.dat"), b"not a world").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, game.join("saves").join("linked")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            game.join("saves").join("real"),
            game.join("saves").join("alias"),
        )
        .unwrap();
        let worlds = scan_worlds(&game, &icons);
        assert_eq!(worlds.len(), 1, "only the real dir scans, got {worlds:?}");
        assert_eq!(worlds[0].dir_name, "real");
        #[cfg(unix)]
        {
            assert!(
                WorldPaths::for_world(&game, "linked").is_err(),
                "a symlink out of saves is never a write target"
            );
            assert!(
                WorldPaths::for_world(&game, "alias").is_err(),
                "even a symlink staying inside saves is rejected"
            );
            assert!(WorldPaths::for_world(&game, "real").is_ok());
        }
    }
}
