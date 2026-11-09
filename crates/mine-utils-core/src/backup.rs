//! Timestamped backups of `options.txt` under the app data dir.
//!
//! Backups live at `<data_dir>/backups/<game-dir-hash>/<timestamp>-options.txt`.
//! The timestamp is UTC with nanosecond precision, so names sort
//! lexicographically and stay unique across rapid writes. Each write keeps the
//! newest `Config.backups.keep` files, plus the backup it just wrote and, for a
//! restore, the restore source, so a restore at `keep = 1` can leave two.

use std::path::{Path, PathBuf};

use crate::config::BackupsConfig;
use crate::error::AppError;

/// Timestamped backup storage shared with the `level.dat` backups: naming,
/// unique writes, rotation, listing, and the guarded restore flow. Each caller
/// supplies its file name shape and its restore validation.
pub(crate) mod core {
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use sha2::Digest;

    use crate::config::BackupsConfig;
    use crate::error::AppError;

    /// One listed backup file.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Stored {
        pub file_name: String,
        pub bytes: u64,
        pub display: String,
    }

    /// What a caller's backup names look like.
    pub trait Scheme {
        fn is_kind(&self, file_name: &str) -> bool;
        fn display(&self, file_name: &str) -> String;
        fn error(&self, reason: String) -> AppError;
    }

    /// A scheme that can mint a new backup name.
    pub trait Naming: Scheme {
        fn name(&self, stamp: &str) -> String;
    }

    pub enum NameError {
        Unsafe,
        WrongKind,
        NotOwned,
        Missing,
        NotFile,
    }

    pub struct Restored {
        pub backup: Option<Stored>,
        pub skipped: bool,
    }

    /// The backup folder for one game dir: a stable hash of its canonical path.
    pub fn root_for(data_dir: &Path, game_dir: &Path) -> PathBuf {
        let canonical = std::fs::canonicalize(game_dir).unwrap_or_else(|_| game_dir.to_path_buf());
        let mut hasher = sha2::Sha256::new();
        hasher.update(canonical.to_string_lossy().as_bytes());
        let digest = hex::encode(hasher.finalize());
        data_dir.join("backups").join(&digest[..16])
    }

    /// Copy `previous` bytes into a new timestamped backup and rotate the
    /// scheme's owned files. When backups are disabled the intent is skipped.
    pub fn create<S: Naming>(
        root: &Path,
        previous: &[u8],
        config: &BackupsConfig,
        scheme: &S,
        owned: impl Fn(&str) -> bool,
    ) -> Result<Option<Stored>, AppError> {
        create_protected(root, previous, config, scheme, owned, &[])
    }

    fn create_protected<S: Naming>(
        root: &Path,
        previous: &[u8],
        config: &BackupsConfig,
        scheme: &S,
        owned: impl Fn(&str) -> bool,
        protected: &[&str],
    ) -> Result<Option<Stored>, AppError> {
        if !config.enabled {
            return Ok(None);
        }
        std::fs::create_dir_all(root)
            .map_err(|err| scheme.error(format!("{}: {err}", root.display())))?;
        let stored = write_unique(root, previous, scheme)?;
        let mut protected = protected.to_vec();
        protected.push(&stored.file_name);
        rotate(root, config.keep.max(1), scheme, &owned, &protected)?;
        Ok(Some(stored))
    }

    /// List the scheme's owned backups, newest first.
    pub fn list<S: Scheme>(
        root: &Path,
        scheme: &S,
        owned: impl Fn(&str) -> bool,
    ) -> Result<Vec<Stored>, AppError> {
        let mut names = names(root, scheme, &owned)?;
        names.sort();
        names.reverse();
        Ok(names
            .into_iter()
            .filter_map(|file_name| {
                let path = root.join(&file_name);
                let bytes = std::fs::symlink_metadata(&path).ok()?.len();
                Some(Stored {
                    display: scheme.display(&file_name),
                    file_name,
                    bytes,
                })
            })
            .collect())
    }

    /// Resolve one backup name inside `root`, rejecting unsafe names, names of
    /// the wrong kind, names this scheme does not own, and symlinks.
    pub fn validated_name<S: Scheme>(
        root: &Path,
        file_name: &str,
        scheme: &S,
        owned: impl Fn(&str) -> bool,
    ) -> Result<PathBuf, NameError> {
        if file_name.is_empty()
            || file_name == "."
            || file_name == ".."
            || file_name.contains('/')
            || file_name.contains('\\')
            || file_name.contains('\0')
        {
            return Err(NameError::Unsafe);
        }
        if !scheme.is_kind(file_name) {
            return Err(NameError::WrongKind);
        }
        if !owned(file_name) {
            return Err(NameError::NotOwned);
        }
        let path = root.join(file_name);
        let metadata = std::fs::symlink_metadata(&path).map_err(|_| NameError::Missing)?;
        if !metadata.is_file() {
            return Err(NameError::NotFile);
        }
        Ok(path)
    }

    pub struct RestoreFiles<'a> {
        pub root: &'a Path,
        pub source: &'a Path,
        pub target: &'a Path,
        pub file_name: &'a str,
    }

    /// Restore one validated backup over its target. The current bytes are
    /// backed up first, then the caller's guard runs immediately before the
    /// write.
    pub fn restore<S, V, W, G>(
        files: RestoreFiles<'_>,
        config: &BackupsConfig,
        scheme: &S,
        owned: impl Fn(&str) -> bool,
        validate: V,
        write: W,
        before_write: G,
    ) -> Result<Restored, AppError>
    where
        S: Naming,
        V: FnOnce(&str, &[u8]) -> Result<(), AppError>,
        W: FnOnce(&[u8]) -> Result<(), AppError>,
        G: FnOnce() -> Result<(), AppError>,
    {
        let bytes = std::fs::read(files.source).map_err(|err| scheme.error(format!("{err}")))?;
        validate(files.file_name, &bytes)?;

        let read_target = || match std::fs::read(files.target) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(AppError::Io(err)),
        };
        let current = read_target()?;
        let backup = match &current {
            Some(current) => create_protected(
                files.root,
                current,
                config,
                scheme,
                owned,
                &[files.file_name],
            )?,
            None => None,
        };

        before_write()?;
        if read_target()? != current {
            return Err(AppError::Stack(
                crate::resource_stack::StackError::StaleStack,
            ));
        }
        write(&bytes)?;
        Ok(Restored {
            backup,
            skipped: !config.enabled,
        })
    }

    fn names<S: Scheme>(
        root: &Path,
        scheme: &S,
        owned: &impl Fn(&str) -> bool,
    ) -> Result<Vec<String>, AppError> {
        let entries = match std::fs::read_dir(root) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(scheme.error(format!("{}: {err}", root.display()))),
        };
        Ok(entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                // `file_type` never follows a symlink, so a link to a regular
                // file is not listed as a restorable backup.
                if !entry.file_type().ok()?.is_file() {
                    return None;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                owned(&name).then_some(name)
            })
            .collect())
    }

    /// Remove the oldest unprotected backups until at most `keep` remain, if possible.
    fn rotate<S: Scheme>(
        root: &Path,
        keep: u32,
        scheme: &S,
        owned: &impl Fn(&str) -> bool,
        protected: &[&str],
    ) -> Result<(), AppError> {
        let mut names = names(root, scheme, owned)?;
        names.sort();
        let excess = names.len().saturating_sub(keep as usize);
        for name in names
            .into_iter()
            .filter(|name| !protected.contains(&name.as_str()))
            .take(excess)
        {
            std::fs::remove_file(root.join(name)).map_err(|err| scheme.error(err.to_string()))?;
        }
        sync_dir(root);
        Ok(())
    }

    /// Write to a hidden temp file, fsync it, then publish it with `hard_link`
    /// to the timestamped name and fsync the directory. `hard_link` fails with
    /// `AlreadyExists` instead of replacing a file another process published,
    /// so a backup name is never clobbered; on a collision the temp is removed
    /// and the next timestamp is tried.
    fn write_unique<S: Naming>(
        root: &Path,
        previous: &[u8],
        scheme: &S,
    ) -> Result<Stored, AppError> {
        for _ in 0..64 {
            let nanos = unique_nanos();
            let stamp = timestamp(nanos);
            let file_name = scheme.name(&stamp);
            let path = root.join(&file_name);
            let tmp = root.join(format!(".{stamp}.{}.tmp", std::process::id()));
            let opened = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp);
            let mut file = match opened {
                Ok(file) => file,
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(err) => return Err(scheme.error(format!("{}: {err}", tmp.display()))),
            };
            let written = (|| -> std::io::Result<()> {
                file.write_all(previous)?;
                file.sync_all()?;
                Ok(())
            })();
            if let Err(err) = written {
                let _ = std::fs::remove_file(&tmp);
                return Err(scheme.error(format!("{}: {err}", tmp.display())));
            }
            match std::fs::hard_link(&tmp, &path) {
                Ok(()) => {
                    let _ = std::fs::remove_file(&tmp);
                    sync_dir(root);
                    return Ok(Stored {
                        display: scheme.display(&file_name),
                        file_name,
                        bytes: previous.len() as u64,
                    });
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    let _ = std::fs::remove_file(&tmp);
                    continue;
                }
                Err(err) => {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(scheme.error(format!("{}: {err}", path.display())));
                }
            }
        }
        Err(scheme.error("could not find a free backup name after 64 tries".to_string()))
    }

    fn sync_dir(path: &Path) {
        if let Ok(handle) = std::fs::File::open(path) {
            let _ = handle.sync_all();
        }
    }

    /// Nanoseconds since the epoch, strictly increasing within this process so
    /// rapid writes always get distinct, ordered timestamps.
    pub(super) fn unique_nanos() -> u64 {
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

    /// `YYYYMMDDTHHMMSS.nnnnnnnnn` in UTC: sorts chronologically as text.
    pub(super) fn timestamp(nanos: u64) -> String {
        let seconds = (nanos / 1_000_000_000) as i64;
        let fraction = nanos % 1_000_000_000;
        let (year, month, day, hour, minute, second) = civil(seconds);
        format!("{year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}.{fraction:09}")
    }

    /// `YYYY-MM-DD HH:MM:SS UTC`, derived from the stamp. Every slice uses
    /// `get`, so a crafted non-ASCII name falls back instead of panicking.
    pub fn display_stamp(stamp: &str) -> String {
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
    pub(super) fn civil(seconds: i64) -> (i64, u32, u32, u32, u32, u32) {
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
}

const OPTIONS_SUFFIX: &str = "-options.txt";
const OPTIONS: OptionsScheme = OptionsScheme;

struct OptionsScheme;

impl core::Scheme for OptionsScheme {
    fn is_kind(&self, file_name: &str) -> bool {
        file_name.ends_with(OPTIONS_SUFFIX)
    }

    fn display(&self, file_name: &str) -> String {
        core::display_stamp(file_name.strip_suffix(OPTIONS_SUFFIX).unwrap_or(file_name))
    }

    fn error(&self, reason: String) -> AppError {
        backup_error(reason)
    }
}

impl core::Naming for OptionsScheme {
    fn name(&self, stamp: &str) -> String {
        format!("{stamp}{OPTIONS_SUFFIX}")
    }
}

fn options_owned(file_name: &str) -> bool {
    file_name.ends_with(OPTIONS_SUFFIX)
}

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

fn backup_info(stored: core::Stored) -> BackupInfo {
    BackupInfo {
        file_name: stored.file_name,
        bytes: stored.bytes,
        display: stored.display,
    }
}

/// The backup folder for one game dir: a stable hash of its canonical path.
pub fn root_for(data_dir: &Path, game_dir: &Path) -> PathBuf {
    core::root_for(data_dir, game_dir)
}

/// Copy `previous` bytes into a new timestamped backup and rotate. When the
/// config disables backups this records the intent and skips the write.
pub fn create(
    root: &Path,
    previous: &[u8],
    config: &BackupsConfig,
) -> Result<BackupOutcome, AppError> {
    let stored = core::create(root, previous, config, &OPTIONS, options_owned)?;
    Ok(BackupOutcome {
        skipped: stored.is_none(),
        created: stored.map(backup_info),
    })
}

/// List the backups of one game dir, newest first.
pub fn list(root: &Path) -> Result<Vec<BackupInfo>, AppError> {
    Ok(core::list(root, &OPTIONS, options_owned)?
        .into_iter()
        .map(backup_info)
        .collect())
}

/// Restore one backup over `options.txt`. The name must resolve inside the
/// game dir's own backup folder. The backup bytes must parse as an
/// `options.txt` before anything is replaced. The current bytes are backed up
/// first, then a guard runs immediately before the atomic write. A restored
/// file may recreate a missing `options.txt`; that is the point of a backup.
pub fn restore(
    root: &Path,
    options_path: &Path,
    file_name: &str,
    config: &BackupsConfig,
) -> Result<BackupOutcome, AppError> {
    restore_guarded(root, options_path, file_name, config, || Ok(()))
}

/// `restore` with a caller-supplied guard that runs after the pre-restore
/// backup exists and immediately before the current file is replaced. The stack
/// path uses it to re-check the game lock after the backup step.
pub fn restore_guarded<F>(
    root: &Path,
    options_path: &Path,
    file_name: &str,
    config: &BackupsConfig,
    before_write: F,
) -> Result<BackupOutcome, AppError>
where
    F: FnOnce() -> Result<(), AppError>,
{
    crate::options_txt::reject_symlink(options_path)?;
    let source = validated_backup(root, file_name)?;
    let restored = core::restore(
        core::RestoreFiles {
            root,
            source: &source,
            target: options_path,
            file_name,
        },
        config,
        &OPTIONS,
        options_owned,
        validate_backup_bytes,
        |bytes| crate::options_txt::write_options_unchecked(options_path, bytes),
        before_write,
    )?;
    Ok(BackupOutcome {
        created: restored.backup.map(backup_info),
        skipped: restored.skipped,
    })
}

/// A backup is only restorable when it is a readable ASCII/UTF-8 `options.txt`
/// with a `resourcePacks` array. Anything else is rejected before the current
/// file is touched.
fn validate_backup_bytes(file_name: &str, bytes: &[u8]) -> Result<(), AppError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| backup_error(format!("{file_name} is not valid UTF-8")))?;
    crate::options_txt::OptionsTxt::parse(text)
        .map_err(|err| backup_error(format!("{file_name} is not a valid options.txt: {err}")))?;
    Ok(())
}

/// Read one backup after validating it lives in the given backup folder.
pub fn read(root: &Path, file_name: &str) -> Result<Vec<u8>, AppError> {
    let source = validated_backup(root, file_name)?;
    std::fs::read(&source).map_err(|err| backup_error(format!("{err}")))
}

fn validated_backup(root: &Path, file_name: &str) -> Result<PathBuf, AppError> {
    core::validated_name(root, file_name, &OPTIONS, options_owned).map_err(|err| match err {
        core::NameError::Unsafe => backup_error(format!(
            "{file_name} is not a backup name in {}",
            root.display()
        )),
        core::NameError::WrongKind | core::NameError::NotOwned => {
            backup_error(format!("{file_name} is not an options.txt backup"))
        }
        core::NameError::Missing => backup_error(format!("no backup named {file_name}")),
        core::NameError::NotFile => backup_error(format!(
            "{file_name} is not a backup file in {}",
            root.display()
        )),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_preserves_new_backup_with_future_dated_files() {
        let root = tempfile::tempdir().unwrap();
        for name in [
            "99980101T000000.000000000-options.txt",
            "99990101T000000.000000000-options.txt",
        ] {
            std::fs::write(root.path().join(name), b"future").unwrap();
        }
        let created = create(root.path(), b"new", &config(2))
            .unwrap()
            .created
            .unwrap();
        assert_eq!(
            std::fs::read(root.path().join(created.file_name)).unwrap(),
            b"new"
        );
        assert_eq!(list(root.path()).unwrap().len(), 2);
        assert!(root
            .path()
            .join("99990101T000000.000000000-options.txt")
            .exists());
    }

    #[test]
    fn failed_restore_preserves_source_at_capacity() {
        let root = tempfile::tempdir().unwrap();
        let options = root.path().join("options.txt");
        let name = "20000101T000000.000000000-options.txt";
        std::fs::write(root.path().join(name), b"resourcePacks:[\"old\"]\n").unwrap();
        std::fs::write(&options, b"resourcePacks:[\"current\"]\n").unwrap();
        let err = restore_guarded(root.path(), &options, name, &config(1), || {
            Err(backup_error("game started"))
        })
        .unwrap_err();
        assert_eq!(err.code(), "backup");
        assert_eq!(
            std::fs::read(root.path().join(name)).unwrap(),
            b"resourcePacks:[\"old\"]\n"
        );
        assert_eq!(
            std::fs::read(options).unwrap(),
            b"resourcePacks:[\"current\"]\n"
        );
        assert_eq!(list(root.path()).unwrap().len(), 2);
    }

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
            let stamp = core::timestamp(core::unique_nanos());
            assert!(stamp > previous, "{stamp} must sort after {previous}");
            previous = stamp;
        }
    }

    #[test]
    fn timestamp_math_is_correct() {
        assert_eq!(core::civil(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(core::civil(1_792_933_140), (2026, 10, 25, 12, 59, 0));
        let stamp = core::timestamp(core::unique_nanos());
        assert!(stamp.starts_with("2026"), "{stamp}");
    }

    #[test]
    fn display_timestamp_never_panics_on_non_ascii_names() {
        let name = "202é610T120000.000000000-options.txt";
        let display = core::display_stamp(name.strip_suffix(OPTIONS_SUFFIX).unwrap_or(name));
        assert!(
            !display.contains("UTC"),
            "a bad boundary falls back: {display}"
        );

        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(name), b"body").unwrap();
        let listed = list(root.path()).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].display, display);
    }

    #[test]
    fn rotation_keeps_the_newest_backups() {
        let root = tempfile::tempdir().unwrap();
        let keep = 3;
        for round in 0..10 {
            create(
                root.path(),
                format!("resourcePacks:[\"v{round}\"]\n").as_bytes(),
                &config(keep),
            )
            .unwrap();
        }

        let listed = list(root.path()).unwrap();
        assert_eq!(listed.len() as u32, keep, "only the newest survive");
        assert_eq!(
            std::fs::read(root.path().join(&listed[0].file_name)).unwrap(),
            b"resourcePacks:[\"v9\"]\n",
            "the newest backup is the last write"
        );
    }

    #[test]
    fn keep_is_clamped_to_at_least_one() {
        let root = tempfile::tempdir().unwrap();
        create(root.path(), b"resourcePacks:[\"a\"]\n", &config(0)).unwrap();
        create(root.path(), b"resourcePacks:[\"b\"]\n", &config(0)).unwrap();

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

        let outcome = create(root.path(), b"resourcePacks:[]\n", &disabled).unwrap();

        assert!(outcome.skipped);
        assert_eq!(outcome.created, None);
        assert!(list(root.path()).unwrap().is_empty());
    }

    #[test]
    fn restore_puts_the_old_bytes_back_and_backs_up_the_current() {
        let root = tempfile::tempdir().unwrap();
        let options = root.path().join("options.txt");
        std::fs::write(&options, b"resourcePacks:[\"v1\"]\n").unwrap();

        let first = create(root.path(), b"resourcePacks:[\"v1\"]\n", &config(10)).unwrap();
        std::fs::write(&options, b"resourcePacks:[\"v2\"]\n").unwrap();

        let outcome = restore(
            root.path(),
            &options,
            &first.created.unwrap().file_name,
            &config(10),
        )
        .unwrap();

        assert_eq!(
            std::fs::read(&options).unwrap(),
            b"resourcePacks:[\"v1\"]\n"
        );
        assert!(outcome.created.is_some(), "the current bytes are backed up");
        let listed = list(root.path()).unwrap();
        assert!(
            listed.iter().any(|info| {
                std::fs::read(root.path().join(&info.file_name)).unwrap()
                    == b"resourcePacks:[\"v2\"]\n"
            }),
            "the pre-restore bytes survive as a backup"
        );
    }

    #[test]
    fn restore_recreates_a_missing_options_file_from_a_valid_backup() {
        let root = tempfile::tempdir().unwrap();
        let options = root.path().join("options.txt");
        let source = b"resourcePacks:[\"vanilla\"]\nlang:en_us\n";
        let first = create(root.path(), source, &config(10)).unwrap();
        let _ = std::fs::remove_file(&options);

        let outcome = restore(
            root.path(),
            &options,
            &first.created.unwrap().file_name,
            &config(10),
        )
        .unwrap();

        assert_eq!(std::fs::read(&options).unwrap(), source);
        assert_eq!(
            outcome.created, None,
            "there was no current file to back up"
        );
    }

    #[test]
    fn malformed_backup_is_rejected_and_leaves_the_current_file() {
        let root = tempfile::tempdir().unwrap();
        let options = root.path().join("options.txt");
        std::fs::write(&options, b"resourcePacks:[\"current\"]\n").unwrap();
        let name = "20260101T000000.000000000-options.txt";
        std::fs::write(root.path().join(name), b"this is not options.txt").unwrap();

        let err = restore(root.path(), &options, name, &config(10)).unwrap_err();

        assert_eq!(err.code(), "backup");
        assert!(err.to_string().contains("not a valid options.txt"), "{err}");
        assert_eq!(
            std::fs::read(&options).unwrap(),
            b"resourcePacks:[\"current\"]\n"
        );
    }

    #[test]
    fn restore_rejects_an_edit_during_guard() {
        let root = tempfile::tempdir().unwrap();
        let options = root.path().join("options.txt");
        std::fs::write(&options, b"resourcePacks:[\"current\"]\n").unwrap();
        let first = create(root.path(), b"resourcePacks:[\"old\"]\n", &config(10)).unwrap();
        let err = restore_guarded(
            root.path(),
            &options,
            &first.created.unwrap().file_name,
            &config(10),
            || {
                std::fs::write(&options, b"resourcePacks:[\"current\"]\nlang:de_de\n").unwrap();
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(err.code(), "stale_stack");
        assert_eq!(
            std::fs::read(&options).unwrap(),
            b"resourcePacks:[\"current\"]\nlang:de_de\n"
        );
        assert_eq!(list(root.path()).unwrap().len(), 2);
    }

    #[test]
    fn restore_guard_runs_after_the_backup_and_before_the_write() {
        let root = tempfile::tempdir().unwrap();
        let options = root.path().join("options.txt");
        std::fs::write(&options, b"resourcePacks:[\"current\"]\n").unwrap();
        let first = create(root.path(), b"resourcePacks:[\"old\"]\n", &config(10)).unwrap();
        let name = first.created.unwrap().file_name;

        let err = restore_guarded(root.path(), &options, &name, &config(10), || {
            assert_eq!(
                std::fs::read(&options).unwrap(),
                b"resourcePacks:[\"current\"]\n",
                "the current file is untouched when the guard runs"
            );
            assert_eq!(
                list(root.path()).unwrap().len(),
                2,
                "the pre-restore backup exists before the guard"
            );
            Err(backup_error("game started"))
        })
        .unwrap_err();

        assert_eq!(err.code(), "backup");
        assert_eq!(
            std::fs::read(&options).unwrap(),
            b"resourcePacks:[\"current\"]\n",
            "a blocked restore leaves the old bytes in place"
        );
    }

    #[test]
    fn missing_backup_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        let options = root.path().join("options.txt");
        std::fs::write(&options, b"resourcePacks:[\"current\"]\n").unwrap();

        let err = restore(
            root.path(),
            &options,
            "20260101T000000.000000000-options.txt",
            &config(10),
        )
        .unwrap_err();

        assert_eq!(err.code(), "backup");
        assert_eq!(
            std::fs::read(&options).unwrap(),
            b"resourcePacks:[\"current\"]\n"
        );
    }

    #[test]
    fn restore_rejects_names_outside_the_backup_folder() {
        let root = tempfile::tempdir().unwrap();
        let options = root.path().join("options.txt");
        std::fs::write(&options, b"resourcePacks:[\"current\"]\n").unwrap();
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
        assert_eq!(
            std::fs::read(&options).unwrap(),
            b"resourcePacks:[\"current\"]\n"
        );
    }

    #[test]
    fn restore_rejects_parent_and_nested_names_that_exist() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("backups");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        let options = dir.path().join("options.txt");
        std::fs::write(&options, b"resourcePacks:[\"current\"]\n").unwrap();
        std::fs::write(
            dir.path().join("x-options.txt"),
            b"resourcePacks:[\"parent\"]\n",
        )
        .unwrap();
        std::fs::write(
            root.join("sub").join("x-options.txt"),
            b"resourcePacks:[\"nested\"]\n",
        )
        .unwrap();

        for name in ["../x-options.txt", "sub/x-options.txt"] {
            let err = restore(&root, &options, name, &config(10)).unwrap_err();
            assert_eq!(err.code(), "backup", "{name} must be rejected");
        }
        assert_eq!(
            std::fs::read(&options).unwrap(),
            b"resourcePacks:[\"current\"]\n"
        );
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
        create(root.path(), b"resourcePacks:[\"abc\"]\n", &config(10)).unwrap();

        let listed = list(root.path()).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].bytes, 22);
        assert!(listed[0].file_name.ends_with("-options.txt"));
        assert!(
            listed[0].display.ends_with(" UTC"),
            "display timestamp: {}",
            listed[0].display
        );
    }

    #[test]
    fn backup_write_is_atomic_and_leaves_no_temp_files() {
        let root = tempfile::tempdir().unwrap();
        let body = b"resourcePacks:[\"vanilla\"]\n";
        let created = create(root.path(), body, &config(10))
            .unwrap()
            .created
            .unwrap();

        assert_eq!(
            std::fs::read(root.path().join(&created.file_name)).unwrap(),
            body
        );
        let leftovers: Vec<String> = std::fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "no temp file survives: {leftovers:?}");

        // A crash-shaped leftover must not be listed as a backup.
        std::fs::write(root.path().join(".20260101T000000.000000000.1.tmp"), b"x").unwrap();
        assert_eq!(list(root.path()).unwrap().len(), 1);
    }

    #[test]
    fn rapid_backups_get_distinct_names_and_leave_no_temp_files() {
        let root = tempfile::tempdir().unwrap();
        let first = create(root.path(), b"resourcePacks:[\"one\"]\n", &config(10))
            .unwrap()
            .created
            .unwrap();
        let second = create(root.path(), b"resourcePacks:[\"two\"]\n", &config(10))
            .unwrap()
            .created
            .unwrap();

        assert_ne!(
            first.file_name, second.file_name,
            "a rapid second write publishes under its own name"
        );
        assert_eq!(
            std::fs::read(root.path().join(&first.file_name)).unwrap(),
            b"resourcePacks:[\"one\"]\n"
        );
        assert_eq!(
            std::fs::read(root.path().join(&second.file_name)).unwrap(),
            b"resourcePacks:[\"two\"]\n"
        );
        let leftovers: Vec<String> = std::fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "no temp file survives: {leftovers:?}");
    }

    #[test]
    fn symlinked_backups_are_not_listed() {
        let root = tempfile::tempdir().unwrap();
        let real = create(root.path(), b"resourcePacks:[\"x\"]\n", &config(10))
            .unwrap()
            .created
            .unwrap();
        let link_name = "20260101T000000.000000000-options.txt";
        std::os::unix::fs::symlink(
            root.path().join(&real.file_name),
            root.path().join(link_name),
        )
        .unwrap();

        let listed = list(root.path()).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].file_name, real.file_name);
        assert!(
            restore(
                root.path(),
                &root.path().join("options.txt"),
                link_name,
                &config(10)
            )
            .is_err(),
            "a symlink is not a restorable backup"
        );
    }
}
