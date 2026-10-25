//! Timestamped backups of `options.txt` under the app data dir.
//!
//! Backups live at `<data_dir>/backups/<game-dir-hash>/<timestamp>-options.txt`.
//! The timestamp is UTC with nanosecond precision, so names sort
//! lexicographically and stay unique across rapid writes. Only the newest
//! `Config.backups.keep` files survive each write.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::Digest;

use crate::config::BackupsConfig;
use crate::error::AppError;

const SUFFIX: &str = "-options.txt";
const KEEP_AT_LEAST: u32 = 1;

/// One backup file as the UI lists it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BackupInfo {
    pub file_name: String,
    pub bytes: u64,
    pub display: String,
}

/// What a backup attempt did. When backups are disabled the intent is
/// recorded and no file is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupOutcome {
    pub created: Option<BackupInfo>,
    pub skipped: bool,
}

fn backup_error(reason: impl std::fmt::Display) -> AppError {
    AppError::Backup {
        reason: reason.to_string(),
    }
}

/// The backup folder for one game dir: a stable hash of its canonical path.
pub fn root_for(data_dir: &Path, game_dir: &Path) -> PathBuf {
    let canonical = std::fs::canonicalize(game_dir).unwrap_or_else(|_| game_dir.to_path_buf());
    let mut hasher = sha2::Sha256::new();
    hasher.update(canonical.to_string_lossy().as_bytes());
    let digest = hex::encode(hasher.finalize());
    data_dir.join("backups").join(&digest[..16])
}

/// Copy `previous` bytes into a new timestamped backup and rotate. When the
/// config disables backups this records the intent and skips the write.
pub fn create(
    root: &Path,
    previous: &[u8],
    config: &BackupsConfig,
) -> Result<BackupOutcome, AppError> {
    if !config.enabled {
        return Ok(BackupOutcome {
            created: None,
            skipped: true,
        });
    }
    std::fs::create_dir_all(root)
        .map_err(|err| backup_error(format!("{}: {err}", root.display())))?;
    let info = write_unique(root, previous)?;
    rotate(root, config.keep.max(KEEP_AT_LEAST))?;
    Ok(BackupOutcome {
        created: Some(info),
        skipped: false,
    })
}

/// List the backups of one game dir, newest first.
pub fn list(root: &Path) -> Result<Vec<BackupInfo>, AppError> {
    let mut names = backup_names(root)?;
    names.sort();
    names.reverse();
    Ok(names
        .into_iter()
        .filter_map(|file_name| {
            let path = root.join(&file_name);
            let bytes = std::fs::metadata(&path).ok()?.len();
            Some(BackupInfo {
                display: display_timestamp(&file_name),
                file_name,
                bytes,
            })
        })
        .collect())
}

/// Restore one backup over `options.txt`. The name must resolve inside the
/// game dir's own backup folder. The current bytes are backed up first, then
/// the backup is written atomically. A restored file may recreate a missing
/// `options.txt`; that is the point of a backup.
pub fn restore(
    root: &Path,
    options_path: &Path,
    file_name: &str,
    config: &BackupsConfig,
) -> Result<BackupOutcome, AppError> {
    let source = validated_backup(root, file_name)?;
    let bytes = std::fs::read(&source).map_err(|err| backup_error(format!("{err}")))?;

    let outcome = match std::fs::read(options_path) {
        Ok(current) => create(root, &current, config)?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => BackupOutcome {
            created: None,
            skipped: !config.enabled,
        },
        Err(err) => return Err(AppError::Io(err)),
    };

    crate::options_txt::write_options_unchecked(options_path, &bytes)?;
    Ok(outcome)
}

/// Read one backup after validating it lives in the given backup folder.
pub fn read(root: &Path, file_name: &str) -> Result<Vec<u8>, AppError> {
    let source = validated_backup(root, file_name)?;
    std::fs::read(&source).map_err(|err| backup_error(format!("{err}")))
}

fn validated_backup(root: &Path, file_name: &str) -> Result<PathBuf, AppError> {
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
    if !file_name.ends_with(SUFFIX) {
        return Err(backup_error(format!(
            "{file_name} is not an options.txt backup"
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

fn backup_names(root: &Path) -> Result<Vec<String>, AppError> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(backup_error(format!("{}: {err}", root.display()))),
    };
    Ok(entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_backup = name.ends_with(SUFFIX) && entry.path().is_file();
            is_backup.then_some(name)
        })
        .collect())
}

/// Keep only the newest `keep` backups. Names sort chronologically.
fn rotate(root: &Path, keep: u32) -> Result<(), AppError> {
    let mut names = backup_names(root)?;
    names.sort();
    let excess = names.len().saturating_sub(keep as usize);
    for name in names.into_iter().take(excess) {
        std::fs::remove_file(root.join(name)).map_err(|err| backup_error(format!("{err}")))?;
    }
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

fn write_unique(root: &Path, previous: &[u8]) -> Result<BackupInfo, AppError> {
    for _ in 0..64 {
        let nanos = unique_nanos();
        let stamp = timestamp(nanos);
        let file_name = format!("{stamp}{SUFFIX}");
        let path = root.join(&file_name);
        let opened = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path);
        let mut file = match opened {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(backup_error(format!("{}: {err}", path.display()))),
        };
        let written = (|| -> std::io::Result<u64> {
            file.write_all(previous)?;
            file.sync_all()?;
            Ok(previous.len() as u64)
        })();
        return match written {
            Ok(bytes) => Ok(BackupInfo {
                display: display_timestamp(&file_name),
                file_name,
                bytes,
            }),
            Err(err) => {
                let _ = std::fs::remove_file(&path);
                Err(backup_error(format!("{}: {err}", path.display())))
            }
        };
    }
    Err(backup_error(
        "could not find a free backup name after 64 tries",
    ))
}

/// `YYYYMMDDTHHMMSS.nnnnnnnnn` in UTC: sorts chronologically as text.
fn timestamp(nanos: u64) -> String {
    let seconds = (nanos / 1_000_000_000) as i64;
    let fraction = nanos % 1_000_000_000;
    let (year, month, day, hour, minute, second) = civil(seconds);
    format!("{year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}.{fraction:09}")
}

/// `YYYY-MM-DD HH:MM:SS UTC`, derived from the backup file name.
fn display_timestamp(file_name: &str) -> String {
    let stamp = file_name.strip_suffix(SUFFIX).unwrap_or(file_name);
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
    format!(
        "{}-{}-{} {}:{}:{} UTC",
        &date[0..4],
        &date[4..6],
        &date[6..8],
        &time[0..2],
        &time[2..4],
        &time[4..6]
    )
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

    fn config(keep: u32) -> BackupsConfig {
        BackupsConfig {
            enabled: true,
            keep,
        }
    }

    #[test]
    fn timestamps_sort_and_stay_unique_under_rapid_writes() {
        let mut previous = String::new();
        for _ in 0..50 {
            let stamp = timestamp(unique_nanos());
            assert!(stamp > previous, "{stamp} must sort after {previous}");
            previous = stamp;
        }
    }

    #[test]
    fn timestamp_math_is_correct() {
        assert_eq!(civil(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil(1_792_933_140), (2026, 10, 25, 12, 59, 0));
        let stamp = timestamp(unique_nanos());
        assert!(stamp.starts_with("2026"), "{stamp}");
    }

    #[test]
    fn rotation_keeps_the_newest_backups() {
        let root = tempfile::tempdir().unwrap();
        let keep = 3;
        for round in 0..10 {
            create(
                root.path(),
                format!("bytes {round}").as_bytes(),
                &config(keep),
            )
            .unwrap();
        }

        let listed = list(root.path()).unwrap();
        assert_eq!(listed.len() as u32, keep, "only the newest survive");
        assert_eq!(
            std::fs::read(root.path().join(&listed[0].file_name)).unwrap(),
            b"bytes 9",
            "the newest backup is the last write"
        );
    }

    #[test]
    fn keep_is_clamped_to_at_least_one() {
        let root = tempfile::tempdir().unwrap();
        create(root.path(), b"first", &config(0)).unwrap();
        create(root.path(), b"second", &config(0)).unwrap();

        let listed = list(root.path()).unwrap();
        assert_eq!(listed.len(), 1);
    }

    #[test]
    fn disabled_backups_record_the_intent_and_skip_the_write() {
        let root = tempfile::tempdir().unwrap();
        let disabled = BackupsConfig {
            enabled: false,
            keep: 10,
        };

        let outcome = create(root.path(), b"bytes", &disabled).unwrap();

        assert!(outcome.skipped);
        assert_eq!(outcome.created, None);
        assert!(list(root.path()).unwrap().is_empty());
    }

    #[test]
    fn restore_puts_the_old_bytes_back_and_backs_up_the_current() {
        let root = tempfile::tempdir().unwrap();
        let options = root.path().join("options.txt");
        std::fs::write(&options, b"v1").unwrap();

        let first = create(root.path(), b"v1", &config(10)).unwrap();
        std::fs::write(&options, b"v2").unwrap();

        let outcome = restore(
            root.path(),
            &options,
            &first.created.unwrap().file_name,
            &config(10),
        )
        .unwrap();

        assert_eq!(std::fs::read(&options).unwrap(), b"v1");
        assert!(outcome.created.is_some(), "the current bytes are backed up");
        let listed = list(root.path()).unwrap();
        assert!(
            listed
                .iter()
                .any(|info| std::fs::read(root.path().join(&info.file_name)).unwrap() == b"v2"),
            "the pre-restore bytes survive as a backup"
        );
    }

    #[test]
    fn missing_backup_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        let options = root.path().join("options.txt");
        std::fs::write(&options, b"current").unwrap();

        let err = restore(
            root.path(),
            &options,
            "20260101T000000.000000000-options.txt",
            &config(10),
        )
        .unwrap_err();

        assert_eq!(err.code(), "backup");
        assert_eq!(std::fs::read(&options).unwrap(), b"current");
    }

    #[test]
    fn restore_rejects_names_outside_the_backup_folder() {
        let root = tempfile::tempdir().unwrap();
        let options = root.path().join("options.txt");
        std::fs::write(&options, b"current").unwrap();
        std::fs::write(root.path().join("sneaky-options.txt"), b"stolen").unwrap();

        for name in [
            "../sneaky-options.txt",
            "sub/dir-options.txt",
            "..-options.txt",
            "plain.txt",
            "",
        ] {
            let err = restore(root.path(), &options, name, &config(10)).unwrap_err();
            assert_eq!(err.code(), "backup", "{name} must be rejected");
        }
        assert_eq!(std::fs::read(&options).unwrap(), b"current");
    }

    #[test]
    fn root_for_is_stable_per_game_dir() {
        let data = tempfile::tempdir().unwrap();
        let game = tempfile::tempdir().unwrap();

        let first = root_for(data.path(), game.path());
        let second = root_for(data.path(), game.path());
        assert_eq!(first, second);
        assert!(first.starts_with(data.path().join("backups")));

        let other = tempfile::tempdir().unwrap();
        assert_ne!(first, root_for(data.path(), other.path()));
    }

    #[test]
    fn list_reports_names_sizes_and_display_timestamps() {
        let root = tempfile::tempdir().unwrap();
        create(root.path(), b"12345", &config(10)).unwrap();

        let listed = list(root.path()).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].bytes, 5);
        assert!(listed[0].file_name.ends_with("-options.txt"));
        assert!(
            listed[0].display.ends_with(" UTC"),
            "display timestamp: {}",
            listed[0].display
        );
    }
}
