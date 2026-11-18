//! Timestamped backups of `level.dat` under the app data dir.
//!
//! Backups live at `<data_dir>/backups/<game-dir-hash>/<timestamp>-<world>-level.dat`.
//! The timestamp is UTC with nanosecond precision, so names sort
//! lexicographically and stay unique across rapid writes. Each write keeps the
//! newest `Config.backups.keep` files per world, plus the backup it just wrote
//! and, for a restore, the restore source. The world directory name is
//! safe and unique within one game dir.
//!
//! A backup is only restorable when it decompresses as gzip and holds NBT with
//! `Data.DataPacks`. If both world seeds are readable and differ, restore
//! refuses without changing anything. A missing or corrupt current file, or an
//! unreadable seed, still permits restore.

use std::path::{Path, PathBuf};

use crate::backup::core as backup_core;
use crate::config::BackupsConfig;
use crate::error::AppError;
use crate::level_dat;

const LEVEL_SUFFIX: &str = "-level.dat";

struct LevelScheme<'a> {
    world: &'a str,
}

impl backup_core::Scheme for LevelScheme<'_> {
    fn is_kind(&self, file_name: &str) -> bool {
        is_level_backup(file_name)
    }

    fn display(&self, file_name: &str) -> String {
        level_display(file_name)
    }

    fn error(&self, reason: String) -> AppError {
        backup_error(reason)
    }
}

impl backup_core::Naming for LevelScheme<'_> {
    fn name(&self, stamp: &str) -> String {
        format!("{stamp}-{}-level.dat", self.world)
    }
}

struct AllLevels;

impl backup_core::Scheme for AllLevels {
    fn is_kind(&self, file_name: &str) -> bool {
        is_level_backup(file_name)
    }

    fn display(&self, file_name: &str) -> String {
        level_display(file_name)
    }

    fn error(&self, reason: String) -> AppError {
        backup_error(reason)
    }
}

fn is_level_backup(name: &str) -> bool {
    name.ends_with(LEVEL_SUFFIX)
}

fn level_display(file_name: &str) -> String {
    backup_core::display_stamp(
        file_name
            .split_once('-')
            .map(|(stamp, _)| stamp)
            .unwrap_or(file_name),
    )
}

fn world_of(file_name: &str) -> Option<&str> {
    let rest = file_name.strip_suffix(LEVEL_SUFFIX)?;
    // The stamp is `YYYYMMDDTHHMMSS.fraction`, which holds no `-`, so the
    // first `-` separates the stamp from the world name.
    let (stamp, world) = rest.split_once('-')?;
    if stamp.len() < 9 || world.is_empty() || world.contains('/') {
        return None;
    }
    Some(world)
}

/// One `level.dat` backup as the UI lists it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "wire-types", derive(ts_rs::TS))]
pub struct LevelBackupInfo {
    pub file_name: String,
    pub world: String,
    pub bytes: u64,
    pub display: String,
}

/// What a backup attempt did. When backups are disabled the intent is recorded
/// and no file is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelBackupOutcome {
    pub created: Option<LevelBackupInfo>,
    pub skipped: bool,
}

fn backup_error(reason: impl std::fmt::Display) -> AppError {
    AppError::LevelBackup {
        reason: reason.to_string(),
    }
}

fn level_info(stored: backup_core::Stored, world: &str) -> LevelBackupInfo {
    LevelBackupInfo {
        file_name: stored.file_name,
        world: world.to_string(),
        bytes: stored.bytes,
        display: stored.display,
    }
}

/// The backup folder for one game dir: a stable hash of its canonical path.
/// Shared with the `options.txt` backups, so one folder holds both kinds.
pub fn root_for(data_dir: &Path, game_dir: &Path) -> PathBuf {
    backup_core::root_for(data_dir, game_dir)
}

/// Copy `previous` bytes into a new timestamped backup and rotate that
/// world's backups. When the config disables backups this records the intent
/// and skips the write.
pub fn create(
    root: &Path,
    world: &str,
    previous: &[u8],
    config: &BackupsConfig,
) -> Result<LevelBackupOutcome, AppError> {
    let scheme = LevelScheme { world };
    let stored = backup_core::create(root, previous, config, &scheme, |name| {
        world_of(name) == Some(world)
    })?;
    Ok(LevelBackupOutcome {
        skipped: stored.is_none(),
        created: stored.map(|stored| level_info(stored, world)),
    })
}

/// List one world's backups, newest first.
pub fn list(root: &Path, world: &str) -> Result<Vec<LevelBackupInfo>, AppError> {
    let scheme = LevelScheme { world };
    Ok(
        backup_core::list(root, &scheme, |name| world_of(name) == Some(world))?
            .into_iter()
            .map(|stored| level_info(stored, world))
            .collect(),
    )
}

/// List every `level.dat` backup in the folder, newest first, for the UI
/// section that groups backups by world.
pub fn list_all(root: &Path) -> Result<Vec<LevelBackupInfo>, AppError> {
    Ok(backup_core::list(root, &AllLevels, is_level_backup)?
        .into_iter()
        .filter_map(|stored| {
            // Stray names with no world (a bare `-level.dat`) form no group,
            // so they are skipped instead of surfacing as an empty row.
            let world = world_of(&stored.file_name)?.to_string();
            Some(LevelBackupInfo {
                display: stored.display,
                world,
                file_name: stored.file_name,
                bytes: stored.bytes,
            })
        })
        .collect())
}

/// Restore one backup over `level.dat`. The name must resolve inside the
/// game dir's own backup folder and belong to this world. The backup bytes
/// must parse as a `level.dat` with `Data.DataPacks` before anything is
/// replaced. Different readable world seeds refuse the restore without changing
/// anything. A missing or corrupt current file, or an unreadable seed, still
/// permits restore. The current bytes are backed up first, then a guard runs
/// immediately before the atomic write.
pub fn restore_guarded<F>(
    root: &Path,
    world: &str,
    level_dat_path: &Path,
    file_name: &str,
    config: &BackupsConfig,
    before_write: F,
) -> Result<LevelBackupOutcome, AppError>
where
    F: FnOnce() -> Result<(), AppError>,
{
    let source = validated_backup(root, world, file_name)?;
    let scheme = LevelScheme { world };
    let restored = backup_core::restore(
        backup_core::RestoreFiles {
            root,
            source: &source,
            target: level_dat_path,
            file_name,
        },
        config,
        &scheme,
        |name| world_of(name) == Some(world),
        |name, bytes| {
            validate_backup_bytes(name, bytes)?;
            match std::fs::read(level_dat_path) {
                Ok(current) => {
                    if matches!(
                        (level_dat::world_seed(&current), level_dat::world_seed(bytes)),
                        (Some(a), Some(b)) if a != b
                    ) {
                        return Err(AppError::WorldMismatch);
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(AppError::Io(err)),
            }
            Ok(())
        },
        |bytes| {
            level_dat::write_level_dat(level_dat_path, bytes)?;
            Ok(())
        },
        before_write,
    )?;
    Ok(LevelBackupOutcome {
        created: restored.backup.map(|stored| level_info(stored, world)),
        skipped: restored.skipped,
    })
}

/// A backup is only restorable when it decompresses as gzip and parses as a
/// `level.dat` with datapack lists. Anything else is rejected before the
/// current file is touched.
fn validate_backup_bytes(file_name: &str, bytes: &[u8]) -> Result<(), AppError> {
    level_dat::parse(bytes)
        .map_err(|err| backup_error(format!("{file_name} is not a valid level.dat: {err}")))?;
    Ok(())
}

fn validated_backup(root: &Path, world: &str, file_name: &str) -> Result<PathBuf, AppError> {
    let scheme = LevelScheme { world };
    backup_core::validated_name(root, file_name, &scheme, |name| {
        world_of(name) == Some(world)
    })
    .map_err(|err| match err {
        backup_core::NameError::Unsafe => backup_error(format!(
            "{file_name} is not a backup name in {}",
            root.display()
        )),
        backup_core::NameError::WrongKind => {
            backup_error(format!("{file_name} is not a level.dat backup"))
        }
        backup_core::NameError::NotOwned => {
            backup_error(format!("{file_name} is not a backup of world {world}"))
        }
        backup_core::NameError::Missing => backup_error(format!("no backup named {file_name}")),
        backup_core::NameError::NotFile => backup_error(format!(
            "{file_name} is not a backup file in {}",
            root.display()
        )),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_level_dat() -> Vec<u8> {
        let value = fastnbt::Value::Compound(
            [(
                "Data".to_string(),
                fastnbt::Value::Compound(
                    [
                        (
                            "Version".to_string(),
                            fastnbt::Value::Compound(
                                [(
                                    "Name".to_string(),
                                    fastnbt::Value::String("1.21.10".to_string()),
                                )]
                                .into_iter()
                                .collect(),
                            ),
                        ),
                        (
                            "DataPacks".to_string(),
                            fastnbt::Value::Compound(
                                [
                                    (
                                        "Enabled".to_string(),
                                        fastnbt::Value::List(vec![fastnbt::Value::String(
                                            "vanilla".to_string(),
                                        )]),
                                    ),
                                    ("Disabled".to_string(), fastnbt::Value::List(Vec::new())),
                                ]
                                .into_iter()
                                .collect(),
                            ),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                ),
            )]
            .into_iter()
            .collect(),
        );
        level_dat::gzip(&fastnbt::to_bytes(&value).unwrap())
    }

    fn seeded(seed: i64) -> Vec<u8> {
        let raw = level_dat::gunzip(&fixture_level_dat()).unwrap();
        let mut value: fastnbt::Value = fastnbt::from_bytes(&raw).unwrap();
        if let fastnbt::Value::Compound(root) = &mut value {
            if let Some(fastnbt::Value::Compound(data)) = root.get_mut("Data") {
                data.insert("RandomSeed".into(), fastnbt::Value::Long(seed));
            }
        }
        level_dat::gzip(&fastnbt::to_bytes(&value).unwrap())
    }

    #[test]
    fn mismatch_does_not_back_up_rotate_or_call_before_write() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("backups");
        let level = dir.path().join("level.dat");
        let current = seeded(99);
        let name = create(&root, "world", &seeded(42), &config(1))
            .unwrap()
            .created
            .unwrap()
            .file_name;
        std::fs::write(&level, &current).unwrap();
        let called = std::cell::Cell::new(false);
        let err = restore_guarded(&root, "world", &level, &name, &config(1), || {
            called.set(true);
            Ok(())
        })
        .unwrap_err();
        assert_eq!(err.code(), "world_mismatch");
        let names: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec![name]);
        assert_eq!(std::fs::read(&level).unwrap(), current);
        assert!(!called.get());
    }

    #[test]
    fn restore_checks_world_seed_and_allows_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("backups");
        let level = dir.path().join("level.dat");
        let backup = seeded(42);
        let current = seeded(99);
        let name = create(&root, "world", &backup, &config(10))
            .unwrap()
            .created
            .unwrap()
            .file_name;
        std::fs::write(&level, &current).unwrap();
        let restore = || restore_guarded(&root, "world", &level, &name, &config(10), || Ok(()));
        assert_eq!(restore().unwrap_err().code(), "world_mismatch");
        assert_eq!(std::fs::read(&level).unwrap(), current);
        std::fs::write(&level, &backup).unwrap();
        restore().unwrap();
        assert_eq!(std::fs::read(&level).unwrap(), backup);
        std::fs::remove_file(&level).unwrap();
        restore().unwrap();
        assert_eq!(std::fs::read(&level).unwrap(), backup);
    }

    #[test]
    fn restore_uses_modern_seed_and_allows_unreadable_seeds() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("backups");
        let level = dir.path().join("level.dat");
        let legacy = seeded(42);
        let raw = level_dat::gunzip(&legacy).unwrap();
        let mut value: fastnbt::Value = fastnbt::from_bytes(&raw).unwrap();
        if let fastnbt::Value::Compound(root) = &mut value {
            if let Some(fastnbt::Value::Compound(data)) = root.get_mut("Data") {
                data.insert(
                    "WorldGenSettings".into(),
                    fastnbt::Value::Compound(
                        [("seed".into(), fastnbt::Value::Long(99))]
                            .into_iter()
                            .collect(),
                    ),
                );
            }
        }
        let modern = level_dat::gzip(&fastnbt::to_bytes(&value).unwrap());
        let name = create(&root, "world", &modern, &config(10))
            .unwrap()
            .created
            .unwrap()
            .file_name;
        let restore = || restore_guarded(&root, "world", &level, &name, &config(10), || Ok(()));
        std::fs::write(&level, &legacy).unwrap();
        assert_eq!(restore().unwrap_err().code(), "world_mismatch");
        std::fs::write(&level, fixture_level_dat()).unwrap();
        restore().unwrap();
        assert_eq!(std::fs::read(&level).unwrap(), modern);
        std::fs::write(&level, b"corrupt").unwrap();
        restore().unwrap();
        assert_eq!(std::fs::read(&level).unwrap(), modern);
    }

    fn compound<const N: usize>(pairs: [(&str, fastnbt::Value); N]) -> fastnbt::Value {
        fastnbt::Value::Compound(
            pairs
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect(),
        )
    }

    /// A pre-1.13 `level.dat`: a version name, a seed, and no `Data.DataPacks`.
    fn legacy_level_dat(seed: i64, level_name: &str) -> Vec<u8> {
        let version = compound([("Name", fastnbt::Value::String("1.12.2".to_string()))]);
        let data = compound([
            ("Version", version),
            ("LevelName", fastnbt::Value::String(level_name.to_string())),
            ("RandomSeed", fastnbt::Value::Long(seed)),
        ]);
        level_dat::gzip(&fastnbt::to_bytes(&compound([("Data", data)])).unwrap())
    }

    #[test]
    fn restore_accepts_a_pre_datapacks_backup_over_a_matching_world() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("backups");
        let level = dir.path().join("level.dat");
        let backup = legacy_level_dat(42, "Saved");
        let name = create(&root, "world", &backup, &config(10))
            .unwrap()
            .created
            .unwrap()
            .file_name;
        std::fs::write(&level, legacy_level_dat(42, "Edited")).unwrap();
        restore_guarded(&root, "world", &level, &name, &config(10), || Ok(())).unwrap();
        assert_eq!(std::fs::read(&level).unwrap(), backup);
    }

    #[test]
    fn restore_refuses_a_pre_datapacks_backup_with_a_different_seed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("backups");
        let level = dir.path().join("level.dat");
        let name = create(&root, "world", &legacy_level_dat(42, "Saved"), &config(10))
            .unwrap()
            .created
            .unwrap()
            .file_name;
        let current = legacy_level_dat(99, "Edited");
        std::fs::write(&level, &current).unwrap();
        let err =
            restore_guarded(&root, "world", &level, &name, &config(10), || Ok(())).unwrap_err();
        assert_eq!(err.code(), "world_mismatch");
        assert_eq!(std::fs::read(&level).unwrap(), current);
    }

    #[test]
    fn restore_refuses_a_gzipped_backup_that_is_not_nbt() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("backups");
        std::fs::create_dir_all(&root).unwrap();
        let name = "20260101T000000.000000000-world-level.dat";
        std::fs::write(root.join(name), level_dat::gzip(b"not nbt")).unwrap();
        let level = dir.path().join("level.dat");
        let current = legacy_level_dat(42, "Edited");
        std::fs::write(&level, &current).unwrap();
        let err =
            restore_guarded(&root, "world", &level, name, &config(10), || Ok(())).unwrap_err();
        assert_eq!(err.code(), "level_backup");
        assert_eq!(std::fs::read(&level).unwrap(), current);
    }

    #[test]
    fn restore_refuses_a_backup_whose_datapacks_are_malformed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("backups");
        let level = dir.path().join("level.dat");
        let packs = compound([
            ("Enabled", fastnbt::Value::Int(1)),
            ("Disabled", fastnbt::Value::List(Vec::new())),
        ]);
        let data = compound([
            (
                "Version",
                compound([("Name", fastnbt::Value::String("1.21.10".to_string()))]),
            ),
            ("DataPacks", packs),
        ]);
        let backup = level_dat::gzip(&fastnbt::to_bytes(&compound([("Data", data)])).unwrap());
        let name = create(&root, "world", &backup, &config(10))
            .unwrap()
            .created
            .unwrap()
            .file_name;
        let current = legacy_level_dat(42, "Edited");
        std::fs::write(&level, &current).unwrap();
        let err =
            restore_guarded(&root, "world", &level, &name, &config(10), || Ok(())).unwrap_err();
        assert_eq!(err.code(), "level_backup");
        assert_eq!(std::fs::read(&level).unwrap(), current);
    }

    fn config(keep: u32) -> BackupsConfig {
        BackupsConfig {
            enabled: true,
            keep,
        }
    }

    #[test]
    fn rotation_keeps_the_newest_backups_per_world() {
        let root = tempfile::tempdir().unwrap();
        let body = fixture_level_dat();
        for _ in 0..5 {
            create(root.path(), "alpha", &body, &config(3)).unwrap();
        }
        for _ in 0..2 {
            create(root.path(), "beta", &body, &config(3)).unwrap();
        }
        assert_eq!(list(root.path(), "alpha").unwrap().len(), 3);
        assert_eq!(list(root.path(), "beta").unwrap().len(), 2);
        assert_eq!(list_all(root.path()).unwrap().len(), 5);
    }

    #[test]
    fn restore_replaces_level_dat_after_backing_up_current() {
        let dir = tempfile::tempdir().unwrap();
        let world = dir.path().join("world");
        std::fs::create_dir_all(&world).unwrap();
        let level = world.join("level.dat");
        let first = fixture_level_dat();
        std::fs::write(&level, &first).unwrap();
        let root = dir.path().join("backups");

        let created = create(root.as_path(), "world", &first, &config(10))
            .unwrap()
            .created
            .unwrap();
        let second = level_dat::render_with(&first, &["vanilla".to_string()], &[]).unwrap();
        let second = level_dat::render_with(
            &second,
            &["vanilla".to_string(), "file/a.zip".to_string()],
            &[],
        )
        .unwrap();
        std::fs::write(&level, &second).unwrap();

        restore_guarded(
            root.as_path(),
            "world",
            &level,
            &created.file_name,
            &config(10),
            || Ok(()),
        )
        .unwrap();
        assert_eq!(std::fs::read(&level).unwrap(), first);
    }

    #[test]
    fn restore_accepts_a_world_name_with_dots() {
        let dir = tempfile::tempdir().unwrap();
        let world = dir.path().join("saves").join("Survival..");
        std::fs::create_dir_all(&world).unwrap();
        let level = world.join("level.dat");
        let first = fixture_level_dat();
        std::fs::write(&level, &first).unwrap();
        let root = dir.path().join("backups");

        let created = create(root.as_path(), "Survival..", &first, &config(10))
            .unwrap()
            .created
            .unwrap();
        std::fs::write(&level, b"edited after the backup").unwrap();

        restore_guarded(
            root.as_path(),
            "Survival..",
            &level,
            &created.file_name,
            &config(10),
            || Ok(()),
        )
        .unwrap();
        assert_eq!(std::fs::read(&level).unwrap(), first);
    }

    #[test]
    fn malformed_backup_leaves_current_file() {
        let dir = tempfile::tempdir().unwrap();
        let world = dir.path().join("world");
        std::fs::create_dir_all(&world).unwrap();
        let level = world.join("level.dat");
        let current = fixture_level_dat();
        std::fs::write(&level, &current).unwrap();
        let root = dir.path().join("backups");
        std::fs::create_dir_all(&root).unwrap();
        let name = "20260101T000000.000000000-world-level.dat";
        std::fs::write(root.join(name), b"not nbt").unwrap();

        let err = restore_guarded(
            root.as_path(),
            "world",
            &level,
            name,
            &config(10),
            || Ok(()),
        )
        .unwrap_err();
        assert_eq!(err.code(), "level_backup");
        assert_eq!(std::fs::read(&level).unwrap(), current);
    }

    #[test]
    fn wrong_world_backup_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let level = dir.path().join("level.dat");
        let body = fixture_level_dat();
        let root = dir.path().join("backups");
        let created = create(root.as_path(), "alpha", &body, &config(10))
            .unwrap()
            .created
            .unwrap();
        let err = restore_guarded(
            root.as_path(),
            "beta",
            &level,
            &created.file_name,
            &config(10),
            || Ok(()),
        )
        .unwrap_err();
        assert_eq!(err.code(), "level_backup");
    }

    #[test]
    fn missing_backup_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = restore_guarded(
            dir.path(),
            "world",
            &dir.path().join("level.dat"),
            "20260101T000000.000000000-world-level.dat",
            &config(10),
            || Ok(()),
        )
        .unwrap_err();
        assert_eq!(err.code(), "level_backup");
    }
}
