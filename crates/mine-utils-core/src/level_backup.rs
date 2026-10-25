//! Timestamped backups of `level.dat` under the app data dir.
//!
//! Backups live at `<data_dir>/backups/<game-dir-hash>/<timestamp>-<world>-level.dat`.
//! The timestamp is UTC with nanosecond precision, so names sort
//! lexicographically and stay unique across rapid writes. Only the newest
//! `Config.backups.keep` files survive per world. The world directory name is
//! safe and unique within one game dir.
//!
//! The machinery mirrors `backup.rs`: write to a hidden temp file, `fsync`
//! it, publish it with `hard_link` (no clobber), `fsync` the directory, and
//! rotate. A backup is only restorable when it decompresses as gzip and holds
//! NBT with `Data.DataPacks`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::Digest;

use crate::config::BackupsConfig;
use crate::error::AppError;
use crate::level_dat;

/// One `level.dat` backup as the UI lists it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

/// The backup folder for one game dir: a stable hash of its canonical path.
/// Shared with the `options.txt` backups, so one folder holds both kinds.
pub fn root_for(data_dir: &Path, game_dir: &Path) -> PathBuf {
    let canonical = std::fs::canonicalize(game_dir).unwrap_or_else(|_| game_dir.to_path_buf());
    let mut hasher = sha2::Sha256::new();
    hasher.update(canonical.to_string_lossy().as_bytes());
    let digest = hex::encode(hasher.finalize());
    data_dir.join("backups").join(&digest[..16])
}

fn file_name(stamp: &str, world: &str) -> String {
    format!("{stamp}-{world}-level.dat")
}

fn is_level_backup(name: &str) -> bool {
    name.ends_with("-level.dat")
}

fn world_of(file_name: &str) -> Option<&str> {
    let rest = file_name.strip_suffix("-level.dat")?;
    let stamp_end = rest.find('T').and_then(|t| rest[..t].find('-').map(|_| t));
    let _ = stamp_end;
    // The stamp is `YYYYMMDDTHHMMSS.fraction`, which holds no `-`, so the
    // first `-` separates the stamp from the world name.
    let (stamp, world) = rest.split_once('-')?;
    if stamp.len() < 9 || world.is_empty() || world.contains('/') {
        return None;
    }
    Some(world)
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
    if !config.enabled {
        return Ok(LevelBackupOutcome {
            created: None,
            skipped: true,
        });
    }
    std::fs::create_dir_all(root)
        .map_err(|err| backup_error(format!("{}: {err}", root.display())))?;
    let info = write_unique(root, world, previous)?;
    rotate(root, world, config.keep.max(1))?;
    Ok(LevelBackupOutcome {
        created: Some(info),
        skipped: false,
    })
}

/// List one world's backups, newest first.
pub fn list(root: &Path, world: &str) -> Result<Vec<LevelBackupInfo>, AppError> {
    let mut names = backup_names(root, world)?;
    names.sort();
    names.reverse();
    Ok(names
        .into_iter()
        .filter_map(|file_name| {
            let path = root.join(&file_name);
            let bytes = std::fs::symlink_metadata(&path).ok()?.len();
            Some(LevelBackupInfo {
                display: display_timestamp(&file_name),
                file_name,
                world: world.to_string(),
                bytes,
            })
        })
        .collect())
}

/// List every `level.dat` backup in the folder, newest first, for the UI
/// section that groups backups by world.
pub fn list_all(root: &Path) -> Result<Vec<LevelBackupInfo>, AppError> {
    let mut names = all_backup_names(root)?;
    names.sort();
    names.reverse();
    Ok(names
        .into_iter()
        .filter_map(|file_name| {
            let path = root.join(&file_name);
            let bytes = std::fs::symlink_metadata(&path).ok()?.len();
            Some(LevelBackupInfo {
                display: display_timestamp(&file_name),
                world: world_of(&file_name).unwrap_or("").to_string(),
                file_name,
                bytes,
            })
        })
        .collect())
}

/// Restore one backup over `level.dat`. The name must resolve inside the
/// game dir's own backup folder and belong to this world. The backup bytes
/// must parse as a `level.dat` with `Data.DataPacks` before anything is
/// replaced. The current bytes are backed up first, then a guard runs
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
    let bytes = std::fs::read(&source).map_err(|err| backup_error(format!("{err}")))?;
    validate_backup_bytes(file_name, &bytes)?;

    let outcome = match std::fs::read(level_dat_path) {
        Ok(current) => create(root, world, &current, config)?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => LevelBackupOutcome {
            created: None,
            skipped: !config.enabled,
        },
        Err(err) => return Err(AppError::Io(err)),
    };

    before_write()?;
    level_dat::write_level_dat(
        level_dat_path.parent().unwrap_or_else(|| Path::new(".")),
        &bytes,
    )?;
    Ok(outcome)
}

/// A backup is only restorable when it decompresses as gzip and parses as a
/// `level.dat` with datapack lists. Anything else is rejected before the
/// current file is touched.
fn validate_backup_bytes(file_name: &str, bytes: &[u8]) -> Result<(), AppError> {
    level_dat::parse(bytes).map_err(|err| {
        backup_error(format!("{file_name} is not a valid level.dat: {err}"))
    })?;
    Ok(())
}

fn validated_backup(root: &Path, world: &str, file_name: &str) -> Result<PathBuf, AppError> {
    if file_name.is_empty()
        || file_name == "."
        || file_name == ".."
        || file_name.contains("..")
        || file_name.contains('/')
        || file_name.contains('\\')
        || file_name.contains('\0')
    {
        return Err(backup_error(format!(
            "{file_name} is not a backup name in {}",
            root.display()
        )));
    }
    if !is_level_backup(file_name) {
        return Err(backup_error(format!(
            "{file_name} is not a level.dat backup"
        )));
    }
    if world_of(file_name) != Some(world) {
        return Err(backup_error(format!(
            "{file_name} is not a backup of world {world}"
        )));
    }
    let path = root.join(file_name);
    let metadata = std::fs::symlink_metadata(&path)
        .map_err(|_| backup_error(format!("no backup named {file_name}")))?;
    if !metadata.is_file() {
        return Err(backup_error(format!(
            "{file_name} is not a backup file in {}",
            root.display()
        )));
    }
    Ok(path)
}

fn backup_names(root: &Path, world: &str) -> Result<Vec<String>, AppError> {
    Ok(all_backup_names(root)?
        .into_iter()
        .filter(|name| world_of(name) == Some(world))
        .collect())
}

fn all_backup_names(root: &Path) -> Result<Vec<String>, AppError> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(backup_error(format!("{}: {err}", root.display()))),
    };
    Ok(entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            if !entry.file_type().ok()?.is_file() {
                return None;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            is_level_backup(&name).then_some(name)
        })
        .collect())
}

/// Keep only the newest `keep` backups of one world. Names sort
/// chronologically, and the world's own names sort together because the stamp
/// leads.
fn rotate(root: &Path, world: &str, keep: u32) -> Result<(), AppError> {
    let mut names = backup_names(root, world)?;
    names.sort();
    let excess = names.len().saturating_sub(keep as usize);
    for name in names.into_iter().take(excess) {
        std::fs::remove_file(root.join(name)).map_err(|err| backup_error(format!("{err}")))?;
    }
    sync_dir(root);
    Ok(())
}

/// Nanoseconds since the epoch, strictly increasing within this process so
/// rapid writes always get distinct, ordered timestamps.
fn unique_nanos() -> u64 {
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0);
    let mut last = LAST.load(Ordering::SeqCst);
    loop {
        let next = now.max(last.saturating_add(1));
        match LAST.compare_exchange(last, next, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return next,
            Err(current) => last = current,
        }
    }
}

/// Write to a hidden temp file, fsync it, then publish it with `hard_link` to
/// the timestamped name and fsync the directory. `hard_link` fails with
/// `AlreadyExists` instead of replacing a file another process published, so a
/// backup name is never clobbered; on a collision the temp is removed and the
/// next timestamp is tried.
fn write_unique(root: &Path, world: &str, previous: &[u8]) -> Result<LevelBackupInfo, AppError> {
    for _ in 0..64 {
        let nanos = unique_nanos();
        let stamp = timestamp(nanos);
        let name = file_name(&stamp, world);
        let path = root.join(&name);
        let tmp = root.join(format!(".{stamp}.{}.tmp", std::process::id()));
        let opened = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp);
        let mut file = match opened {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(backup_error(format!("{}: {err}", tmp.display()))),
        };
        let written = (|| -> std::io::Result<()> {
            file.write_all(previous)?;
            file.sync_all()?;
            Ok(())
        })();
        if let Err(err) = written {
            let _ = std::fs::remove_file(&tmp);
            return Err(backup_error(format!("{}: {err}", tmp.display())));
        }
        match std::fs::hard_link(&tmp, &path) {
            Ok(()) => {
                let _ = std::fs::remove_file(&tmp);
                sync_dir(root);
                return Ok(LevelBackupInfo {
                    display: display_timestamp(&name),
                    file_name: name,
                    world: world.to_string(),
                    bytes: previous.len() as u64,
                });
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                let _ = std::fs::remove_file(&tmp);
                continue;
            }
            Err(err) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(backup_error(format!("{}: {err}", path.display())));
            }
        }
    }
    Err(backup_error(
        "could not find a free backup name after 64 tries",
    ))
}

fn sync_dir(path: &Path) {
    if let Ok(handle) = std::fs::File::open(path) {
        let _ = handle.sync_all();
    }
}

/// `YYYYMMDDTHHMMSS.nnnnnnnnn` in UTC: sorts chronologically as text.
fn timestamp(nanos: u64) -> String {
    let seconds = (nanos / 1_000_000_000) as i64;
    let fraction = nanos % 1_000_000_000;
    let (year, month, day, hour, minute, second) = civil(seconds);
    format!("{year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}.{fraction:09}")
}

/// `YYYY-MM-DD HH:MM:SS UTC`, derived from the backup file name. Every slice
/// uses `get`, so a crafted non-ASCII name falls back instead of panicking.
fn display_timestamp(file_name: &str) -> String {
    let stamp = file_name.split_once('-').map(|(stamp, _)| stamp).unwrap_or(file_name);
    let Some(rest) = stamp.split_once('T') else {
        return stamp.to_string();
    };
    let (date, rest) = rest;
    if date.len() != 8 {
        return stamp.to_string();
    }
    let Some(time) = rest.split_once('.').map(|(time, _)| time) else {
        return stamp.to_string();
    };
    if time.len() != 6 {
        return stamp.to_string();
    }
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) = (
        date.get(0..4),
        date.get(4..6),
        date.get(6..8),
        time.get(0..2),
        time.get(2..4),
        time.get(4..6),
    ) else {
        return stamp.to_string();
    };
    format!("{year}-{month}-{day} {hour}:{minute}:{second} UTC")
}

/// Civil date and time from Unix seconds. Howard Hinnant's days-from-civil,
/// inverted.
fn civil(seconds: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = seconds.div_euclid(86_400);
    let time = seconds.rem_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (
        year,
        month,
        day,
        (time / 3600) as u32,
        ((time % 3600) / 60) as u32,
        (time % 60) as u32,
    )
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
                                    (
                                        "Disabled".to_string(),
                                        fastnbt::Value::List(Vec::new()),
                                    ),
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
        let second = level_dat::set_enabled(&first, "a.zip", true).unwrap();
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

        let err = restore_guarded(root.as_path(), "world", &level, name, &config(10), || Ok(()))
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
