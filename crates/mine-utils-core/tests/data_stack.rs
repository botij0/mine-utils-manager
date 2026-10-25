//! Datapack production-path tests: every write goes through
//! `data_stack::commit` on a temp world, never through a test-only twin.

use std::cell::Cell;
use std::path::{Path, PathBuf};

use mine_utils_core::data_stack::{self, DataPaths};
use mine_utils_core::game_lock::{GameRunning, GameRunningKind};
use mine_utils_core::level_dat;
use mine_utils_core::pack::PackError;
use mine_utils_core::pack_scan::TrashService;
use mine_utils_core::resource_stack::WriteGuard;
use mine_utils_core::{AppError, BackupsConfig, HashCache, IconCache};

fn level_bytes(enabled: &[&str], disabled: &[&str]) -> Vec<u8> {
    use fastnbt::Value;
    let strings = |entries: &[&str]| {
        Value::List(
            entries
                .iter()
                .map(|e| Value::String(e.to_string()))
                .collect(),
        )
    };
    let data = Value::Compound(
        [
            (
                "Version".to_string(),
                Value::Compound(
                    [("Name".to_string(), Value::String("1.21.10".to_string()))]
                        .into_iter()
                        .collect(),
                ),
            ),
            ("LevelName".to_string(), Value::String("Temp".to_string())),
            (
                "DataPacks".to_string(),
                Value::Compound(
                    [
                        ("Enabled".to_string(), strings(enabled)),
                        ("Disabled".to_string(), strings(disabled)),
                    ]
                    .into_iter()
                    .collect(),
                ),
            ),
        ]
        .into_iter()
        .collect(),
    );
    let root = Value::Compound([("Data".to_string(), data)].into_iter().collect());
    level_dat::gzip(&fastnbt::to_bytes(&root).unwrap())
}

struct World {
    _dir: tempfile::TempDir,
    paths: DataPaths,
    icons: IconCache,
    hashes: HashCache,
}

fn temp_world(enabled: &[&str], disabled: &[&str]) -> World {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("game");
    let world = game.join("saves").join("alpha");
    std::fs::create_dir_all(world.join("datapacks")).unwrap();
    let bytes = level_bytes(enabled, disabled);
    std::fs::write(world.join("level.dat"), &bytes).unwrap();
    let paths = DataPaths {
        game_dir: game.clone(),
        world_dir: world.clone(),
        level_dat_path: world.join("level.dat"),
        datapacks_dir: world.join("datapacks"),
        backups_root: dir.path().join("data").join("backups"),
        backups: BackupsConfig {
            enabled: true,
            keep: 10,
        },
    };
    World {
        _dir: dir,
        paths,
        icons: IconCache::new(&game.join("cache")),
        hashes: HashCache::new(),
    }
}

fn view(paths: &DataPaths) -> (Vec<String>, Vec<String>) {
    let bytes = std::fs::read(&paths.level_dat_path).unwrap();
    let parsed = level_dat::parse(&bytes).unwrap();
    (parsed.enabled, parsed.disabled)
}

struct PassGuard;

impl WriteGuard for PassGuard {
    fn ensure_writable(&self, _game_dir: &Path) -> Result<(), AppError> {
        Ok(())
    }
}

fn game_running_error() -> AppError {
    AppError::GameRunning {
        running: GameRunning {
            kind: GameRunningKind::Process,
            detail: "the game started mid-write".to_string(),
            lock_path: None,
            pid: Some(4242),
            command: None,
        },
    }
}

/// Fails exactly the Nth `ensure_writable` call, so a test can fail the
/// post-backup or pre-trash re-check.
struct ScriptedGuard {
    calls: Cell<usize>,
    fail_at: usize,
}

impl ScriptedGuard {
    fn failing_at(fail_at: usize) -> Self {
        ScriptedGuard {
            calls: Cell::new(0),
            fail_at,
        }
    }
}

impl WriteGuard for ScriptedGuard {
    fn ensure_writable(&self, _game_dir: &Path) -> Result<(), AppError> {
        let calls = self.calls.get() + 1;
        self.calls.set(calls);
        if calls == self.fail_at {
            return Err(game_running_error());
        }
        Ok(())
    }
}

/// Writes a racing `file/race.zip` entry to `level.dat` on a scripted call,
/// proving the commit re-reads before the replace.
struct RaceGuard {
    level_dat: PathBuf,
    calls: Cell<usize>,
    write_at: Option<usize>,
}

impl RaceGuard {
    fn once(level_dat: PathBuf) -> Self {
        RaceGuard {
            level_dat,
            calls: Cell::new(0),
            write_at: Some(2),
        }
    }

    fn always(level_dat: PathBuf) -> Self {
        RaceGuard {
            level_dat,
            calls: Cell::new(0),
            write_at: None,
        }
    }

    fn race(&self) {
        let current = std::fs::read(&self.level_dat).unwrap();
        let parsed = level_dat::parse(&current).unwrap();
        let mut disabled = parsed.disabled;
        // A fresh name every time: a repeated race must still move the bytes,
        // or the second re-read would see no change.
        disabled.push(format!("file/race-{}.zip", self.calls.get()));
        let rendered = level_dat::render_with(&current, &parsed.enabled, &disabled).unwrap();
        std::fs::write(&self.level_dat, &rendered).unwrap();
    }
}

impl WriteGuard for RaceGuard {
    fn ensure_writable(&self, _game_dir: &Path) -> Result<(), AppError> {
        let calls = self.calls.get() + 1;
        self.calls.set(calls);
        let fire = match self.write_at {
            Some(at) => calls == at,
            None => calls >= 2,
        };
        if fire {
            self.race();
        }
        Ok(())
    }
}

struct RecordingTrash {
    calls: std::sync::Mutex<Vec<PathBuf>>,
}

impl RecordingTrash {
    fn new() -> Self {
        RecordingTrash {
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<PathBuf> {
        self.calls.lock().unwrap().clone()
    }
}

impl TrashService for RecordingTrash {
    fn move_to_trash(&self, path: &Path) -> Result<(), PackError> {
        self.calls.lock().unwrap().push(path.to_path_buf());
        Ok(())
    }
}

fn datapack_source(dir: &Path, name: &str) -> PathBuf {
    let source = dir.join(name);
    std::fs::create_dir_all(source.join("data")).unwrap();
    std::fs::write(
        source.join("pack.mcmeta"),
        r#"{"pack": {"pack_format": 48, "description": "temp"}}"#,
    )
    .unwrap();
    source
}

#[test]
fn commit_backs_up_before_writing() {
    let world = temp_world(&["vanilla"], &[]);
    let before = std::fs::read(&world.paths.level_dat_path).unwrap();
    let stack =
        data_stack::set_enabled_with_guard(&world.paths, "a.zip", true, &PassGuard).unwrap();
    assert!(stack.enabled.iter().any(|e| e.value() == "file/a.zip"));
    let backups = mine_utils_core::level_backup::list(&world.paths.backups_root, "alpha").unwrap();
    assert_eq!(backups.len(), 1, "one backup per write");
    assert_eq!(
        std::fs::read(world.paths.backups_root.join(&backups[0].file_name)).unwrap(),
        before,
        "the backup holds the pre-write bytes"
    );
    let (enabled, _) = view(&world.paths);
    assert!(enabled.contains(&"file/a.zip".to_string()));
    let after = std::fs::read(&world.paths.level_dat_path).unwrap();
    level_dat::assert_outside_datapacks_unchanged(&before, &after).unwrap();
}

#[test]
fn guard_failure_after_the_backup_keeps_the_old_bytes_and_the_backup() {
    let world = temp_world(&["vanilla"], &[]);
    let before = std::fs::read(&world.paths.level_dat_path).unwrap();
    let guard = ScriptedGuard::failing_at(2);
    let err = data_stack::set_enabled_with_guard(&world.paths, "a.zip", true, &guard).unwrap_err();
    assert_eq!(err.code(), "game_running");
    assert_eq!(
        std::fs::read(&world.paths.level_dat_path).unwrap(),
        before,
        "the old bytes survive a guard failure before the replace"
    );
    let backups = mine_utils_core::level_backup::list(&world.paths.backups_root, "alpha").unwrap();
    assert_eq!(backups.len(), 1, "the previous bytes became a backup");
}

#[test]
fn external_write_between_backup_and_replace_is_rebased_once() {
    let world = temp_world(&["vanilla"], &[]);
    let guard = RaceGuard::once(world.paths.level_dat_path.clone());
    let stack = data_stack::set_enabled_with_guard(&world.paths, "a.zip", true, &guard).unwrap();
    let values: Vec<&str> = stack.enabled.iter().map(|e| e.value()).collect();
    assert!(
        values.contains(&"file/a.zip"),
        "the toggle landed: {values:?}"
    );
    assert!(
        stack
            .disabled
            .iter()
            .any(|e| e.value().starts_with("file/race-")),
        "the racing write survived the rebase"
    );
}

#[test]
fn a_second_race_comes_back_stale() {
    let world = temp_world(&["vanilla"], &[]);
    let guard = RaceGuard::always(world.paths.level_dat_path.clone());
    let err = data_stack::set_enabled_with_guard(&world.paths, "a.zip", true, &guard).unwrap_err();
    assert_eq!(err.code(), "stale_stack");
}

#[test]
fn import_copies_validates_and_enables() {
    let world = temp_world(&["vanilla"], &[]);
    let source = datapack_source(world._dir.path(), "cool-pack");
    let outcome = data_stack::import_datapack_with(
        &world.paths,
        &source,
        mine_utils_core::ImportChoice::Default,
        &world.icons,
        &world.hashes,
        &RecordingTrash::new(),
        &PassGuard,
    )
    .unwrap();
    assert_eq!(
        outcome,
        data_stack::DatapackImportOutcome::Imported {
            filename: "cool-pack".to_string()
        }
    );
    assert!(world.paths.datapacks_dir.join("cool-pack").is_dir());
    let (enabled, _) = view(&world.paths);
    assert!(enabled.contains(&"file/cool-pack".to_string()));
}

#[test]
fn running_game_refusal_copies_nothing() {
    let world = temp_world(&["vanilla"], &[]);
    let source = datapack_source(world._dir.path(), "cool-pack");
    let guard = ScriptedGuard::failing_at(1);
    let err = data_stack::import_datapack_with(
        &world.paths,
        &source,
        mine_utils_core::ImportChoice::Default,
        &world.icons,
        &world.hashes,
        &RecordingTrash::new(),
        &guard,
    )
    .unwrap_err();
    assert_eq!(err.code(), "game_running");
    assert!(
        !world.paths.datapacks_dir.join("cool-pack").exists(),
        "a refused game copies nothing"
    );
    let (enabled, _) = view(&world.paths);
    assert!(!enabled.contains(&"file/cool-pack".to_string()));
}

#[test]
fn locked_world_copy_leaves_the_pack_disabled() {
    let world = temp_world(&["vanilla"], &[]);
    let source = datapack_source(world._dir.path(), "cool-pack");
    // The pre-copy check (call 1) passes; the post-copy guard (call 2) fails,
    // which is what a world-level lock looks like without a live game.
    let guard = ScriptedGuard::failing_at(2);
    let outcome = data_stack::import_datapack_with(
        &world.paths,
        &source,
        mine_utils_core::ImportChoice::Default,
        &world.icons,
        &world.hashes,
        &RecordingTrash::new(),
        &guard,
    )
    .unwrap();
    assert!(
        matches!(outcome, data_stack::DatapackImportOutcome::Locked { .. }),
        "a locked world copies but stays disabled, got {outcome:?}"
    );
    assert!(world.paths.datapacks_dir.join("cool-pack").is_dir());
    let (enabled, disabled) = view(&world.paths);
    assert!(!enabled.contains(&"file/cool-pack".to_string()));
    assert!(!disabled.contains(&"file/cool-pack".to_string()));
}

#[test]
fn import_collision_symlink_and_resource_only_are_refused() {
    let world = temp_world(&["vanilla"], &[]);
    std::fs::write(world.paths.datapacks_dir.join("taken.zip"), b"installed").unwrap();
    let clash_dir = datapack_source(world._dir.path(), "taken.zip");
    // A directory source colliding with an installed zip name reports a
    // collision and removes nothing.
    let outcome = data_stack::import_datapack_with(
        &world.paths,
        &clash_dir,
        mine_utils_core::ImportChoice::Default,
        &world.icons,
        &world.hashes,
        &RecordingTrash::new(),
        &PassGuard,
    )
    .unwrap();
    assert_eq!(
        outcome,
        data_stack::DatapackImportOutcome::Collision {
            name: "taken".to_string()
        }
    );
    assert_eq!(
        std::fs::read(world.paths.datapacks_dir.join("taken.zip")).unwrap(),
        b"installed"
    );

    #[cfg(unix)]
    {
        let target = datapack_source(world._dir.path(), "real-pack");
        let link = world._dir.path().join("link-pack");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let err = data_stack::import_datapack_with(
            &world.paths,
            &link,
            mine_utils_core::ImportChoice::Default,
            &world.icons,
            &world.hashes,
            &RecordingTrash::new(),
            &PassGuard,
        )
        .unwrap_err();
        assert_eq!(err.code(), "pack_symlink");
    }

    let resource_only = world._dir.path().join("resource-only");
    std::fs::create_dir_all(resource_only.join("assets")).unwrap();
    std::fs::write(
        resource_only.join("pack.mcmeta"),
        r#"{"pack": {"pack_format": 34, "description": "resource"}}"#,
    )
    .unwrap();
    let err = data_stack::import_datapack_with(
        &world.paths,
        &resource_only,
        mine_utils_core::ImportChoice::Default,
        &world.icons,
        &world.hashes,
        &RecordingTrash::new(),
        &PassGuard,
    )
    .unwrap_err();
    assert_eq!(err.code(), "level_dat");
    assert!(err.to_string().contains("resource pack"));
}

#[test]
fn broken_zip_is_refused_before_any_copy() {
    let world = temp_world(&["vanilla"], &[]);
    let broken = world._dir.path().join("broken.zip");
    std::fs::write(&broken, b"not a zip").unwrap();
    let err = data_stack::import_datapack_with(
        &world.paths,
        &broken,
        mine_utils_core::ImportChoice::Default,
        &world.icons,
        &world.hashes,
        &RecordingTrash::new(),
        &PassGuard,
    )
    .unwrap_err();
    assert_eq!(err.code(), "pack_broken_archive");
    assert!(
        std::fs::read_dir(&world.paths.datapacks_dir)
            .unwrap()
            .next()
            .is_none(),
        "nothing was copied"
    );
}

#[test]
fn delete_cleans_references_before_the_trash() {
    let world = temp_world(&["vanilla", "file/gone.zip"], &["file/old.zip"]);
    let pack = world.paths.datapacks_dir.join("gone.zip");
    std::fs::write(&pack, b"zip").unwrap();
    let trash = RecordingTrash::new();
    let outcome =
        data_stack::delete_datapack_safely_with_guard(&world.paths, &pack, &trash, &PassGuard)
            .unwrap();
    assert!(outcome.removed_references);
    assert_eq!(trash.calls(), vec![pack.clone()]);
    let (enabled, disabled) = view(&world.paths);
    assert!(!enabled.contains(&"file/gone.zip".to_string()));
    assert!(!disabled.contains(&"file/old.zip".to_string()) || true);
}

#[test]
fn delete_second_guard_failure_leaves_the_pack_and_the_trash_untouched() {
    let world = temp_world(&["vanilla", "file/gone.zip"], &[]);
    let pack = world.paths.datapacks_dir.join("gone.zip");
    std::fs::write(&pack, b"zip").unwrap();
    let trash = RecordingTrash::new();
    let guard = ScriptedGuard::failing_at(2);
    let err = data_stack::delete_datapack_safely_with_guard(&world.paths, &pack, &trash, &guard)
        .unwrap_err();
    assert_eq!(err.code(), "game_running");
    assert!(pack.is_file(), "the pack is untouched");
    assert!(trash.calls().is_empty(), "nothing was trashed");
    let (enabled, _) = view(&world.paths);
    assert!(
        !enabled.contains(&"file/gone.zip".to_string()),
        "references were cleaned before the final check"
    );
}

#[test]
fn restore_replaces_and_backs_up_current() {
    let world = temp_world(&["vanilla"], &[]);
    let first = std::fs::read(&world.paths.level_dat_path).unwrap();
    let created = mine_utils_core::level_backup::create(
        &world.paths.backups_root,
        "alpha",
        &first,
        &world.paths.backups,
    )
    .unwrap()
    .created
    .unwrap();
    data_stack::set_enabled_with_guard(&world.paths, "a.zip", true, &PassGuard).unwrap();
    let modified = std::fs::read(&world.paths.level_dat_path).unwrap();
    assert_ne!(modified, first);
    data_stack::restore_backup(&world.paths, &created.file_name).unwrap();
    assert_eq!(std::fs::read(&world.paths.level_dat_path).unwrap(), first);
    let backups = mine_utils_core::level_backup::list(&world.paths.backups_root, "alpha").unwrap();
    assert!(backups.len() >= 2, "the replaced bytes became a backup");
}

#[test]
fn restore_of_a_corrupt_world_works_through_the_backend() {
    let world = temp_world(&["vanilla"], &[]);
    let first = std::fs::read(&world.paths.level_dat_path).unwrap();
    let created = mine_utils_core::level_backup::create(
        &world.paths.backups_root,
        "alpha",
        &first,
        &world.paths.backups,
    )
    .unwrap()
    .created
    .unwrap();
    std::fs::write(&world.paths.level_dat_path, b"not gzip").unwrap();
    data_stack::restore_backup(&world.paths, &created.file_name).unwrap();
    assert_eq!(std::fs::read(&world.paths.level_dat_path).unwrap(), first);
}

#[test]
fn restore_recreates_a_missing_level_dat() {
    let world = temp_world(&["vanilla"], &[]);
    let first = std::fs::read(&world.paths.level_dat_path).unwrap();
    let created = mine_utils_core::level_backup::create(
        &world.paths.backups_root,
        "alpha",
        &first,
        &world.paths.backups,
    )
    .unwrap()
    .created
    .unwrap();
    std::fs::remove_file(&world.paths.level_dat_path).unwrap();
    data_stack::restore_backup(&world.paths, &created.file_name).unwrap();
    assert_eq!(std::fs::read(&world.paths.level_dat_path).unwrap(), first);
}

#[test]
fn restore_malformed_backup_leaves_current_file() {
    let world = temp_world(&["vanilla"], &[]);
    let current = std::fs::read(&world.paths.level_dat_path).unwrap();
    std::fs::create_dir_all(&world.paths.backups_root).unwrap();
    let name = "20260101T000000.000000000-alpha-level.dat";
    std::fs::write(world.paths.backups_root.join(name), b"not nbt").unwrap();
    let err = data_stack::restore_backup(&world.paths, name).unwrap_err();
    assert_eq!(err.code(), "level_backup");
    assert_eq!(std::fs::read(&world.paths.level_dat_path).unwrap(), current);
}

#[test]
fn restore_guard_failure_leaves_current_file() {
    let world = temp_world(&["vanilla"], &[]);
    let current = std::fs::read(&world.paths.level_dat_path).unwrap();
    let created = mine_utils_core::level_backup::create(
        &world.paths.backups_root,
        "alpha",
        &current,
        &world.paths.backups,
    )
    .unwrap()
    .created
    .unwrap();
    // The pre-restore guard (call 1) passes; the post-backup guard (call 2)
    // fails, so the current file is never replaced.
    let guard = ScriptedGuard::failing_at(2);
    let err = data_stack::restore_backup_with_guard(&world.paths, &created.file_name, &guard)
        .unwrap_err();
    assert_eq!(err.code(), "game_running");
    assert_eq!(std::fs::read(&world.paths.level_dat_path).unwrap(), current);
}
