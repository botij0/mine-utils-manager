//! Datapack production-path tests: every write goes through
//! `data_stack::commit` on a temp world, never through a test-only twin.

use std::cell::{Cell, RefCell};
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
    let paths = DataPaths::for_world(
        &mine_utils_core::WorldPaths::for_world(&game, "alpha").unwrap(),
        &dir.path().join("data"),
        BackupsConfig {
            enabled: true,
            keep: 10,
        },
    );
    World {
        _dir: dir,
        paths,
        icons: IconCache::new(&game.join("cache")),
        hashes: HashCache::new(),
    }
}

#[test]
fn paths_use_the_validated_world_name_and_level_target() {
    let world = temp_world(&["vanilla"], &[]);
    assert_eq!(world.paths.world_name(), "alpha");
    assert_eq!(
        world.paths.level_dat_path(),
        world.paths.world_dir().join("level.dat")
    );
}

fn view(paths: &DataPaths) -> (Vec<String>, Vec<String>) {
    let bytes = std::fs::read(paths.level_dat_path()).unwrap();
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
    let before = std::fs::read(world.paths.level_dat_path()).unwrap();
    let stack =
        data_stack::set_enabled_with_guard(&world.paths, "a.zip", true, &PassGuard).unwrap();
    assert!(stack.enabled.iter().any(|e| e.value() == "file/a.zip"));
    let backups = mine_utils_core::level_backup::list(world.paths.backups_root(), "alpha").unwrap();
    assert_eq!(backups.len(), 1, "one backup per write");
    assert_eq!(
        std::fs::read(world.paths.backups_root().join(&backups[0].file_name)).unwrap(),
        before,
        "the backup holds the pre-write bytes"
    );
    let (enabled, _) = view(&world.paths);
    assert!(enabled.contains(&"file/a.zip".to_string()));
    let after = std::fs::read(world.paths.level_dat_path()).unwrap();
    level_dat::assert_outside_datapacks_unchanged(&before, &after).unwrap();
}

#[test]
fn guard_failure_after_the_backup_keeps_the_old_bytes_and_the_backup() {
    let world = temp_world(&["vanilla"], &[]);
    let before = std::fs::read(world.paths.level_dat_path()).unwrap();
    let guard = ScriptedGuard::failing_at(2);
    let err = data_stack::set_enabled_with_guard(&world.paths, "a.zip", true, &guard).unwrap_err();
    assert_eq!(err.code(), "game_running");
    assert_eq!(
        std::fs::read(world.paths.level_dat_path()).unwrap(),
        before,
        "the old bytes survive a guard failure before the replace"
    );
    let backups = mine_utils_core::level_backup::list(world.paths.backups_root(), "alpha").unwrap();
    assert_eq!(backups.len(), 1, "the previous bytes became a backup");
}

#[test]
fn external_write_between_backup_and_replace_is_rebased_once() {
    let world = temp_world(&["vanilla"], &[]);
    let guard = RaceGuard::once(world.paths.level_dat_path().to_path_buf());
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
    let guard = RaceGuard::always(world.paths.level_dat_path().to_path_buf());
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
    assert!(world.paths.datapacks_dir().join("cool-pack").is_dir());
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
        !world.paths.datapacks_dir().join("cool-pack").exists(),
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
    assert!(world.paths.datapacks_dir().join("cool-pack").is_dir());
    let (enabled, disabled) = view(&world.paths);
    assert!(!enabled.contains(&"file/cool-pack".to_string()));
    assert!(!disabled.contains(&"file/cool-pack".to_string()));
}

#[test]
fn import_collision_symlink_and_resource_only_are_refused() {
    let world = temp_world(&["vanilla"], &[]);
    std::fs::write(world.paths.datapacks_dir().join("taken.zip"), b"installed").unwrap();
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
        std::fs::read(world.paths.datapacks_dir().join("taken.zip")).unwrap(),
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
        std::fs::read_dir(world.paths.datapacks_dir())
            .unwrap()
            .next()
            .is_none(),
        "nothing was copied"
    );
}

fn lists(enabled: &[&str], disabled: &[&str]) -> (Vec<String>, Vec<String>) {
    let owned = |entries: &[&str]| entries.iter().map(|e| e.to_string()).collect();
    (owned(enabled), owned(disabled))
}

struct FnTrash<F>(F);

impl<F: Fn(&Path) -> Result<(), PackError>> TrashService for FnTrash<F> {
    fn move_to_trash(&self, path: &Path) -> Result<(), PackError> {
        (self.0)(path)
    }
}

fn failing_trash(path: &Path) -> Result<(), PackError> {
    Err(PackError::Trash {
        path: path.to_path_buf(),
        reason: "no usable .Trash-1000 on this mount".to_string(),
    })
}

#[test]
fn delete_disables_the_pack_before_the_trash_and_drops_it_after() {
    let world = temp_world(&["vanilla", "file/gone.zip"], &["file/old.zip"]);
    let pack = world.paths.datapacks_dir().join("gone.zip");
    std::fs::write(&pack, b"zip").unwrap();
    let at_trash = RefCell::new(None);
    let trash = FnTrash(|_: &Path| {
        *at_trash.borrow_mut() = Some(view(&world.paths));
        Ok(())
    });
    let outcome =
        data_stack::delete_datapack_safely_with_guard(&world.paths, &pack, &trash, &PassGuard)
            .unwrap();
    assert!(outcome.removed_references);
    assert_eq!(
        at_trash.into_inner(),
        Some(lists(&["vanilla"], &["file/old.zip", "file/gone.zip"]))
    );
    assert_eq!(view(&world.paths), lists(&["vanilla"], &["file/old.zip"]));
}

#[test]
fn delete_trash_failure_keeps_a_disabled_pack_disabled() {
    let world = temp_world(&["vanilla"], &["file/old.zip"]);
    let pack = world.paths.datapacks_dir().join("old.zip");
    std::fs::write(&pack, b"zip").unwrap();
    let err = data_stack::delete_datapack_safely_with_guard(
        &world.paths,
        &pack,
        &FnTrash(failing_trash),
        &PassGuard,
    )
    .unwrap_err();
    assert_eq!(err.code(), "pack_trash");
    assert!(pack.is_file(), "the pack is still installed");
    assert_eq!(view(&world.paths), lists(&["vanilla"], &["file/old.zip"]));
}

#[test]
fn delete_second_guard_failure_leaves_the_pack_disabled_and_untrashed() {
    let world = temp_world(&["vanilla", "file/gone.zip"], &[]);
    let pack = world.paths.datapacks_dir().join("gone.zip");
    std::fs::write(&pack, b"zip").unwrap();
    let trash = RecordingTrash::new();
    let guard = ScriptedGuard::failing_at(2);
    let err = data_stack::delete_datapack_safely_with_guard(&world.paths, &pack, &trash, &guard)
        .unwrap_err();
    assert_eq!(err.code(), "game_running");
    assert!(pack.is_file(), "the pack is untouched");
    assert!(trash.calls().is_empty(), "nothing was trashed");
    assert_eq!(view(&world.paths), lists(&["vanilla"], &["file/gone.zip"]));
}

#[cfg(unix)]
#[test]
fn delete_succeeds_when_the_cleanup_after_the_trash_fails() {
    let world = temp_world(
        &["vanilla", "file/keep.zip", "file/gone.zip"],
        &["file/old.zip"],
    );
    let pack = world.paths.datapacks_dir().join("gone.zip");
    std::fs::write(&pack, b"zip").unwrap();
    let trash_then_lock_world = FnTrash(|_: &Path| {
        std::fs::create_dir(world.paths.world_dir().join("session.lock")).unwrap();
        Ok(())
    });
    let outcome = data_stack::delete_datapack_safely_with_guard(
        &world.paths,
        &pack,
        &trash_then_lock_world,
        &PassGuard,
    )
    .unwrap();
    assert!(!outcome.removed_references);
    assert_eq!(
        view(&world.paths),
        lists(
            &["vanilla", "file/keep.zip"],
            &["file/old.zip", "file/gone.zip"]
        )
    );
}

fn backed_up_lists(paths: &DataPaths) -> Vec<(Vec<String>, Vec<String>)> {
    mine_utils_core::level_backup::list(paths.backups_root(), "alpha")
        .unwrap()
        .iter()
        .map(|backup| {
            let bytes = std::fs::read(paths.backups_root().join(&backup.file_name)).unwrap();
            let parsed = level_dat::parse(&bytes).unwrap();
            (parsed.enabled, parsed.disabled)
        })
        .collect()
}

fn delete_gone_pack(enabled: &[&str], disabled: &[&str], keep: u32) -> World {
    let mut world = temp_world(enabled, disabled);
    world.paths = DataPaths::for_world(
        &mine_utils_core::WorldPaths::for_world(world.paths.game_dir(), "alpha").unwrap(),
        &world._dir.path().join("data"),
        BackupsConfig {
            enabled: true,
            keep,
        },
    );
    let pack = world.paths.datapacks_dir().join("gone.zip");
    std::fs::write(&pack, b"zip").unwrap();
    data_stack::delete_datapack_safely_with_guard(
        &world.paths,
        &pack,
        &RecordingTrash::new(),
        &PassGuard,
    )
    .unwrap();
    world
}

#[test]
fn delete_of_an_enabled_pack_backs_up_only_the_pre_delete_file() {
    let world = delete_gone_pack(&["vanilla", "file/gone.zip"], &["file/old.zip"], 10);
    assert_eq!(
        backed_up_lists(&world.paths),
        vec![lists(&["vanilla", "file/gone.zip"], &["file/old.zip"])]
    );
}

#[test]
fn delete_with_one_kept_backup_keeps_the_pre_delete_file() {
    let world = delete_gone_pack(&["vanilla", "file/gone.zip"], &["file/old.zip"], 1);
    assert_eq!(
        backed_up_lists(&world.paths),
        vec![lists(&["vanilla", "file/gone.zip"], &["file/old.zip"])]
    );
}

#[test]
fn delete_of_a_disabled_pack_backs_up_the_pre_delete_file() {
    let world = delete_gone_pack(&["vanilla"], &["file/old.zip", "file/gone.zip"], 10);
    assert_eq!(
        backed_up_lists(&world.paths),
        vec![lists(&["vanilla"], &["file/old.zip", "file/gone.zip"])]
    );
}

#[test]
fn delete_of_an_unlisted_pack_backs_up_the_pre_delete_file() {
    let world = temp_world(&["vanilla"], &["file/old.zip"]);
    let pack = world.paths.datapacks_dir().join("gone.zip");
    std::fs::write(&pack, b"zip").unwrap();
    let outcome = data_stack::delete_datapack_safely_with_guard(
        &world.paths,
        &pack,
        &RecordingTrash::new(),
        &PassGuard,
    )
    .unwrap();
    assert!(outcome.removed_references);
    assert_eq!(
        backed_up_lists(&world.paths),
        vec![lists(&["vanilla"], &["file/old.zip"])]
    );
    assert_eq!(view(&world.paths), lists(&["vanilla"], &["file/old.zip"]));
}

#[test]
fn restore_refuses_legacy_current_with_different_seed() {
    use fastnbt::Value;
    let world = temp_world(&["vanilla"], &[]);
    let raw = level_dat::gunzip(&level_bytes(&["vanilla"], &[])).unwrap();
    let mut backup: Value = fastnbt::from_bytes(&raw).unwrap();
    let Value::Compound(root) = &mut backup else {
        unreachable!()
    };
    let Some(Value::Compound(data)) = root.get_mut("Data") else {
        unreachable!()
    };
    data.insert(
        "WorldGenSettings".into(),
        Value::Compound([("seed".into(), Value::Long(42))].into_iter().collect()),
    );
    let backup = level_dat::gzip(&fastnbt::to_bytes(&backup).unwrap());
    let created = mine_utils_core::level_backup::create(
        world.paths.backups_root(),
        "alpha",
        &backup,
        world.paths.backups(),
    )
    .unwrap()
    .created
    .unwrap();
    let current = Value::Compound(
        [(
            "Data".into(),
            Value::Compound(
                [("RandomSeed".into(), Value::Long(99))]
                    .into_iter()
                    .collect(),
            ),
        )]
        .into_iter()
        .collect(),
    );
    let current = level_dat::gzip(&fastnbt::to_bytes(&current).unwrap());
    std::fs::write(world.paths.level_dat_path(), &current).unwrap();
    let err = data_stack::restore_backup(&world.paths, &created.file_name).unwrap_err();
    assert_eq!(err.code(), "world_mismatch");
    assert_eq!(
        std::fs::read(world.paths.level_dat_path()).unwrap(),
        current
    );
}

#[test]
fn restore_replaces_and_backs_up_current() {
    let world = temp_world(&["vanilla"], &[]);
    let first = std::fs::read(world.paths.level_dat_path()).unwrap();
    let created = mine_utils_core::level_backup::create(
        world.paths.backups_root(),
        "alpha",
        &first,
        world.paths.backups(),
    )
    .unwrap()
    .created
    .unwrap();
    data_stack::set_enabled_with_guard(&world.paths, "a.zip", true, &PassGuard).unwrap();
    let modified = std::fs::read(world.paths.level_dat_path()).unwrap();
    assert_ne!(modified, first);
    data_stack::restore_backup(&world.paths, &created.file_name).unwrap();
    assert_eq!(std::fs::read(world.paths.level_dat_path()).unwrap(), first);
    let backups = mine_utils_core::level_backup::list(world.paths.backups_root(), "alpha").unwrap();
    assert!(backups.len() >= 2, "the replaced bytes became a backup");
}

#[test]
fn restore_of_a_corrupt_world_works_through_the_backend() {
    let world = temp_world(&["vanilla"], &[]);
    let first = std::fs::read(world.paths.level_dat_path()).unwrap();
    let created = mine_utils_core::level_backup::create(
        world.paths.backups_root(),
        "alpha",
        &first,
        world.paths.backups(),
    )
    .unwrap()
    .created
    .unwrap();
    std::fs::write(world.paths.level_dat_path(), b"not gzip").unwrap();
    data_stack::restore_backup(&world.paths, &created.file_name).unwrap();
    assert_eq!(std::fs::read(world.paths.level_dat_path()).unwrap(), first);
}

#[test]
fn restore_recreates_a_missing_level_dat() {
    let world = temp_world(&["vanilla"], &[]);
    let first = std::fs::read(world.paths.level_dat_path()).unwrap();
    let created = mine_utils_core::level_backup::create(
        world.paths.backups_root(),
        "alpha",
        &first,
        world.paths.backups(),
    )
    .unwrap()
    .created
    .unwrap();
    std::fs::remove_file(world.paths.level_dat_path()).unwrap();
    data_stack::restore_backup(&world.paths, &created.file_name).unwrap();
    assert_eq!(std::fs::read(world.paths.level_dat_path()).unwrap(), first);
}

#[test]
fn restore_malformed_backup_leaves_current_file() {
    let world = temp_world(&["vanilla"], &[]);
    let current = std::fs::read(world.paths.level_dat_path()).unwrap();
    std::fs::create_dir_all(world.paths.backups_root()).unwrap();
    let name = "20260101T000000.000000000-alpha-level.dat";
    std::fs::write(world.paths.backups_root().join(name), b"not nbt").unwrap();
    let err = data_stack::restore_backup(&world.paths, name).unwrap_err();
    assert_eq!(err.code(), "level_backup");
    assert_eq!(
        std::fs::read(world.paths.level_dat_path()).unwrap(),
        current
    );
}

#[test]
fn restore_guard_failure_leaves_current_file() {
    let world = temp_world(&["vanilla"], &[]);
    let current = std::fs::read(world.paths.level_dat_path()).unwrap();
    let created = mine_utils_core::level_backup::create(
        world.paths.backups_root(),
        "alpha",
        &current,
        world.paths.backups(),
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
    assert_eq!(
        std::fs::read(world.paths.level_dat_path()).unwrap(),
        current
    );
}

/// Spawned helper: holds a POSIX write lock on `$MINE_UTILS_LOCK_HOLDER`
/// until it is killed. POSIX locks are per process, so the holder must be
/// its own process: `F_GETLK` reports our own locks as free.
#[test]
fn lock_holder_helper() {
    let Ok(path) = std::env::var("MINE_UTILS_LOCK_HOLDER") else {
        return;
    };
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .expect("open the lock file");
    let mut lock = libc::flock {
        l_type: libc::F_WRLCK as libc::c_short,
        l_whence: libc::SEEK_SET as libc::c_short,
        l_start: 0,
        l_len: 0,
        l_pid: 0,
    };
    use std::os::unix::io::AsRawFd;
    let result = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETLK, &mut lock) };
    assert_eq!(result, 0, "the helper takes the write lock");
    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

fn spawn_lock_holder(path: &Path) -> std::process::Child {
    std::process::Command::new(std::env::current_exe().expect("current exe"))
        .args(["--exact", "lock_holder_helper", "--nocapture"])
        .env("MINE_UTILS_LOCK_HOLDER", path)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn the lock holder")
}

fn wait_for_world_lock(world_dir: &Path) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if mine_utils_core::game_lock::world_locked(world_dir).is_some() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    false
}

/// Install `name` from a source dir, returning the source for a later Replace.
fn install_pack(world: &World, name: &str) -> PathBuf {
    let source = datapack_source(world._dir.path(), name);
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
    assert!(
        matches!(outcome, data_stack::DatapackImportOutcome::Imported { .. }),
        "setup import works, got {outcome:?}"
    );
    source
}

#[test]
fn replace_commits_the_new_datapack_before_trashing_the_holding_folder() {
    let world = temp_world(&["vanilla"], &[]);
    let source = install_pack(&world, "cool-pack");
    std::fs::write(source.join("data/new.json"), b"new content").unwrap();
    let observed = RefCell::new(None);
    let trash = FnTrash(|holding: &Path| {
        *observed.borrow_mut() = Some((
            std::fs::read(world.paths.datapacks_dir().join("cool-pack/data/new.json")).unwrap(),
            holding
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".mine-utils-replace-cool-pack."),
            holding.join("data/new.json").exists(),
        ));
        std::fs::remove_dir_all(holding).unwrap();
        Ok(())
    });
    let outcome = data_stack::import_datapack_with(
        &world.paths,
        &source,
        mine_utils_core::ImportChoice::Replace,
        &world.icons,
        &world.hashes,
        &trash,
        &PassGuard,
    )
    .unwrap();
    assert_eq!(
        outcome,
        data_stack::DatapackImportOutcome::Imported {
            filename: "cool-pack".to_string()
        }
    );
    assert_eq!(
        observed.into_inner(),
        Some((b"new content".to_vec(), true, false))
    );
    assert_eq!(
        view(&world.paths),
        lists(&["vanilla", "file/cool-pack"], &[])
    );
}

#[test]
fn locked_world_replace_is_refused_before_any_trash() {
    let world = temp_world(&["vanilla"], &[]);
    let source = install_pack(&world, "cool-pack");
    let installed = world.paths.datapacks_dir().join("cool-pack");
    assert!(installed.is_dir(), "setup installed the pack");
    let before = std::fs::read(world.paths.level_dat_path()).unwrap();

    let lock_path = world.paths.world_dir().join("session.lock");
    std::fs::write(&lock_path, b"\x00").unwrap();
    let mut child = spawn_lock_holder(&lock_path);
    assert!(
        wait_for_world_lock(world.paths.world_dir()),
        "a second process holds the world lock"
    );

    let trash = RecordingTrash::new();
    let err = data_stack::import_datapack_with(
        &world.paths,
        &source,
        mine_utils_core::ImportChoice::Replace,
        &world.icons,
        &world.hashes,
        &trash,
        &PassGuard,
    )
    .unwrap_err();
    let _ = child.kill();
    let _ = child.wait();

    assert!(
        err.to_string().contains("locked"),
        "a clear locked/replace error, got {err}"
    );
    assert!(
        installed.is_dir(),
        "the old pack survives the refused Replace"
    );
    assert!(trash.calls().is_empty(), "nothing was trashed");
    assert_eq!(
        std::fs::read(world.paths.level_dat_path()).unwrap(),
        before,
        "level.dat is untouched"
    );
}

#[test]
fn locked_world_collision_replace_is_refused() {
    let world = temp_world(&["vanilla"], &[]);
    install_pack(&world, "clash-pack");
    let installed_pack = world.paths.datapacks_dir().join("clash-pack");
    assert!(installed_pack.is_dir());
    let installed_meta_before = std::fs::read(installed_pack.join("pack.mcmeta")).unwrap();
    let level_before = std::fs::read(world.paths.level_dat_path()).unwrap();

    let lock_path = world.paths.world_dir().join("session.lock");
    std::fs::write(&lock_path, b"\x00").unwrap();
    let mut child = spawn_lock_holder(&lock_path);
    assert!(wait_for_world_lock(world.paths.world_dir()));

    // A colliding source: same file name as the installed pack, different
    // bytes, so `Replace` would trash the installed pack to install it.
    let other_dir = world._dir.path().join("other");
    std::fs::create_dir_all(&other_dir).unwrap();
    let clash = other_dir.join("clash-pack");
    std::fs::create_dir_all(clash.join("data")).unwrap();
    std::fs::write(
        clash.join("pack.mcmeta"),
        r#"{"pack": {"pack_format": 48, "description": "other"}}"#,
    )
    .unwrap();
    let trash = RecordingTrash::new();
    let err = data_stack::import_datapack_with(
        &world.paths,
        &clash,
        mine_utils_core::ImportChoice::Replace,
        &world.icons,
        &world.hashes,
        &trash,
        &PassGuard,
    )
    .unwrap_err();
    let _ = child.kill();
    let _ = child.wait();

    assert!(
        err.to_string().contains("locked"),
        "a clear locked/replace error, got {err}"
    );
    assert!(installed_pack.is_dir(), "the installed pack survives");
    assert_eq!(
        std::fs::read(installed_pack.join("pack.mcmeta")).unwrap(),
        installed_meta_before,
        "the installed bytes are untouched"
    );
    assert_eq!(
        std::fs::read(world.paths.level_dat_path()).unwrap(),
        level_before,
        "level.dat is untouched"
    );
    assert!(trash.calls().is_empty(), "nothing was trashed");
}

#[test]
#[cfg(unix)]
fn symlinked_level_dat_is_never_written() {
    let world = temp_world(&["vanilla"], &["file/a.zip"]);
    let outside = world._dir.path().join("outside.dat");
    std::fs::write(&outside, b"precious").unwrap();
    std::fs::remove_file(world.paths.level_dat_path()).unwrap();
    std::os::unix::fs::symlink(&outside, world.paths.level_dat_path()).unwrap();

    let err =
        data_stack::set_enabled_with_guard(&world.paths, "a.zip", true, &PassGuard).unwrap_err();
    assert_eq!(
        err.code(),
        "world",
        "a symlinked level.dat fails closed, got {err}"
    );
    assert_eq!(
        std::fs::read(&outside).unwrap(),
        b"precious",
        "nothing was written through the link"
    );
    assert!(
        std::fs::symlink_metadata(world.paths.level_dat_path())
            .unwrap()
            .file_type()
            .is_symlink(),
        "the link itself is untouched"
    );
}

#[test]
#[cfg(unix)]
fn symlinked_datapacks_dir_fails_closed() {
    let world = temp_world(&["vanilla"], &[]);
    let outside = world._dir.path().join("outside-packs");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("mine.txt"), b"precious").unwrap();
    std::fs::remove_dir(world.paths.datapacks_dir()).unwrap();
    std::os::unix::fs::symlink(&outside, world.paths.datapacks_dir()).unwrap();

    let err = data_stack::scan_datapacks(world.paths.datapacks_dir(), &world.icons).unwrap_err();
    assert_eq!(
        err.code(),
        "world",
        "a symlinked datapacks dir is not listed, got {err}"
    );

    let source = datapack_source(world._dir.path(), "cool-pack");
    let err = data_stack::import_datapack_with(
        &world.paths,
        &source,
        mine_utils_core::ImportChoice::Default,
        &world.icons,
        &world.hashes,
        &RecordingTrash::new(),
        &PassGuard,
    )
    .unwrap_err();
    assert_eq!(err.code(), "world", "no import follows the link, got {err}");

    let err =
        data_stack::set_enabled_with_guard(&world.paths, "a.zip", true, &PassGuard).unwrap_err();
    assert_eq!(err.code(), "world", "no write follows the link, got {err}");

    assert_eq!(
        std::fs::read(outside.join("mine.txt")).unwrap(),
        b"precious",
        "nothing was written outside the world"
    );
    assert!(
        std::fs::read_dir(&outside).unwrap().count() == 1,
        "nothing was copied outside the world"
    );
}

#[test]
fn replace_backup_failure_keeps_the_original_at_its_name() {
    let world = temp_world(&["vanilla", "file/old"], &[]);
    let old = datapack_source(world.paths.datapacks_dir(), "old");
    let source = datapack_source(world._dir.path(), "new");
    std::fs::create_dir_all(world.paths.backups_root().parent().unwrap()).unwrap();
    std::fs::write(world.paths.backups_root(), b"backup directory blocked").unwrap();
    let before = std::fs::read(world.paths.level_dat_path()).unwrap();
    let trash = FnTrash(|path: &Path| {
        std::fs::remove_dir_all(path).unwrap();
        Ok(())
    });
    let err = data_stack::import_datapack_with(
        &world.paths,
        &source,
        mine_utils_core::ImportChoice::Replace,
        &world.icons,
        &world.hashes,
        &trash,
        &PassGuard,
    )
    .unwrap_err();
    assert_eq!(
        std::fs::read(old.join("pack.mcmeta")).unwrap(),
        br#"{"pack": {"pack_format": 48, "description": "temp"}}"#
    );
    assert_eq!(std::fs::read(world.paths.level_dat_path()).unwrap(), before);
    assert!(world
        .paths
        .datapacks_dir()
        .join("new/pack.mcmeta")
        .is_file());
    assert!(
        err.to_string()
            .contains(&format!("Original pack retained at {}", old.display())),
        "{err}"
    );
    println!(
        "level.dat unchanged; original at {}; new pack installed; {err}",
        old.display()
    );
}

#[test]
fn replace_updates_level_dat_before_trashing_the_original() {
    let world = temp_world(&["vanilla"], &[]);
    datapack_source(world.paths.datapacks_dir(), "old");
    let source = datapack_source(world._dir.path(), "new");
    let observed = RefCell::new(None);
    let trash = FnTrash(|path: &Path| {
        *observed.borrow_mut() = Some(view(&world.paths));
        std::fs::remove_dir_all(path).unwrap();
        Ok(())
    });
    let outcome = data_stack::import_datapack_with(
        &world.paths,
        &source,
        mine_utils_core::ImportChoice::Replace,
        &world.icons,
        &world.hashes,
        &trash,
        &PassGuard,
    )
    .unwrap();
    assert_eq!(
        outcome,
        data_stack::DatapackImportOutcome::Imported {
            filename: "new".into()
        }
    );
    assert_eq!(
        observed.into_inner(),
        Some(lists(&["vanilla", "file/new"], &[]))
    );
}

#[test]
fn stale_unsafe_reference_can_be_disabled_and_removed() {
    let world = temp_world(&["vanilla", "file/sub/x.zip"], &[]);
    data_stack::set_enabled_with_guard(&world.paths, "sub/x.zip", false, &PassGuard).unwrap();
    assert_eq!(
        view(&world.paths),
        (vec!["vanilla".into()], vec!["file/sub/x.zip".into()])
    );
    assert!(data_stack::remove_references(&world.paths, "sub/x.zip").unwrap());
    assert_eq!(view(&world.paths), (vec!["vanilla".into()], vec![]));
}

#[cfg(unix)]
#[test]
fn ambiguous_import_is_refused_before_copy() {
    use std::os::unix::ffi::OsStrExt;
    let world = temp_world(&["vanilla"], &[]);
    let bad = world
        .paths
        .datapacks_dir()
        .join(std::ffi::OsStr::from_bytes(b"Caf\xe9"));
    std::fs::write(&bad, b"bad").unwrap();
    let source = datapack_source(world._dir.path(), "Caf�");
    let error = data_stack::import_datapack_with(
        &world.paths,
        &source,
        mine_utils_core::ImportChoice::Replace,
        &world.icons,
        &world.hashes,
        &RecordingTrash::new(),
        &PassGuard,
    )
    .unwrap_err();
    assert_eq!(error.code(), "pack_non_utf8_name");
    assert!(!world.paths.datapacks_dir().join("Caf�").exists());
    assert_eq!(std::fs::read(bad).unwrap(), b"bad");
}
