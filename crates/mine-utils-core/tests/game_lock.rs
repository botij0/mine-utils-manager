//! Game guard tests: a real second process holds a POSIX write lock, and a
//! fake `/proc` proves the process scan.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use mine_utils_core::backup;
use mine_utils_core::game_lock::{
    check_in, lock_detection_supported, GameRunning, GameRunningKind, GameStatus,
};
use mine_utils_core::pack_import::TrashService;
use mine_utils_core::resource_stack::{self, StackPaths, WriteGuard};
use mine_utils_core::AppError;
use mine_utils_core::BackupsConfig;

/// Spawned helper: takes a POSIX write lock on `$MINE_UTILS_LOCK_HOLDER` and
/// holds it until it is killed. Runs as its own process because POSIX locks
/// are per process; `F_GETLK` reports our own locks as free.
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
    println!("MINE_UTILS_LOCK_HELD");
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn spawn_lock_holder(path: &Path) -> Child {
    Command::new(std::env::current_exe().expect("current exe"))
        .args(["--exact", "lock_holder_helper", "--nocapture"])
        .env("MINE_UTILS_LOCK_HOLDER", path)
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn the lock holder")
}

fn empty_proc_root(dir: &Path) -> PathBuf {
    let proc_root = dir.join("proc");
    std::fs::create_dir_all(&proc_root).unwrap();
    proc_root
}

fn game_dir_with_world(dir: &Path) -> PathBuf {
    let game_dir = dir.join("game");
    std::fs::create_dir_all(game_dir.join("saves").join("world")).unwrap();
    game_dir
}

fn wait_for_running(game_dir: &Path, proc_root: &Path) -> Option<mine_utils_core::GameRunning> {
    wait_for_status(game_dir, proc_root).and_then(|status| status.running)
}

fn wait_for_status(game_dir: &Path, proc_root: &Path) -> Option<GameStatus> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let status = check_in(game_dir, proc_root);
        if status.running.is_some() {
            return Some(status);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

#[test]
fn held_posix_lock_is_detected() {
    if !lock_detection_supported() {
        eprintln!("skipping: POSIX lock probing is unsupported here");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let game_dir = game_dir_with_world(dir.path());
    let lock_path = game_dir.join("saves").join("world").join("session.lock");
    std::fs::write(&lock_path, b"\x00").unwrap();
    let proc_root = empty_proc_root(dir.path());

    let mut child = spawn_lock_holder(&lock_path);
    let running = wait_for_running(&game_dir, &proc_root);
    let _ = child.kill();
    let _ = child.wait();

    let running = running.expect("the held lock is detected");
    assert_eq!(running.kind, GameRunningKind::SessionLock);
    assert_eq!(running.lock_path.as_deref(), Some(lock_path.as_path()));
    assert!(running.pid.unwrap() > 0, "the holder pid is named");
    assert!(
        running.detail.contains("locked by pid"),
        "{}",
        running.detail
    );
}

#[test]
fn stale_lock_file_does_not_block() {
    let dir = tempfile::tempdir().unwrap();
    let game_dir = game_dir_with_world(dir.path());
    let lock_path = game_dir.join("saves").join("world").join("session.lock");
    std::fs::write(&lock_path, b"\x00").unwrap();
    let proc_root = empty_proc_root(dir.path());

    let status = check_in(&game_dir, &proc_root);

    assert!(
        status.running.is_none(),
        "an unlocked session.lock is stale"
    );
    assert_eq!(status.stale_locks, vec![lock_path]);
    assert!(mine_utils_core::game_lock::ensure_writable(&game_dir).is_ok());
}

#[test]
fn process_scan_finds_both_game_dir_forms() {
    for (pid, form) in [(4242, "spaced"), (5150, "equals")] {
        let dir = tempfile::tempdir().unwrap();
        let game_dir = dir.path().join("game");
        std::fs::create_dir_all(&game_dir).unwrap();
        let proc_root = empty_proc_root(dir.path());

        let entry = proc_root.join(pid.to_string());
        std::fs::create_dir_all(&entry).unwrap();
        let target = game_dir.to_string_lossy().into_owned();
        let argv: Vec<String> = match form {
            "spaced" => vec![
                "/usr/bin/java".to_string(),
                "-jar".to_string(),
                "client.jar".to_string(),
                "--gameDir".to_string(),
                target,
            ],
            _ => vec![
                "/usr/bin/java".to_string(),
                "-jar".to_string(),
                "client.jar".to_string(),
                format!("--gameDir={target}"),
            ],
        };
        std::fs::write(entry.join("cmdline"), argv.join("\0") + "\0").unwrap();

        let running = check_in(&game_dir, &proc_root)
            .running
            .unwrap_or_else(|| panic!("the --gameDir {form} form is detected"));
        assert_eq!(running.kind, GameRunningKind::Process);
        assert_eq!(running.pid, Some(pid));
        assert!(running.command.unwrap().contains("client.jar"));
    }
}

#[test]
fn process_scan_ignores_a_wrong_game_dir() {
    let dir = tempfile::tempdir().unwrap();
    let game_dir = dir.path().join("game");
    std::fs::create_dir_all(&game_dir).unwrap();
    let proc_root = empty_proc_root(dir.path());

    let entry = proc_root.join("9999");
    std::fs::create_dir_all(&entry).unwrap();
    std::fs::write(
        entry.join("cmdline"),
        format!(
            "/usr/bin/java\0-jar\0client.jar\0--gameDir\0{}\0",
            dir.path().join("other").display()
        ),
    )
    .unwrap();

    let status = check_in(&game_dir, &proc_root);
    assert!(
        status.running.is_none(),
        "a java process for another game dir is ignored"
    );
}

#[test]
fn process_scan_requires_a_java_process() {
    let dir = tempfile::tempdir().unwrap();
    let game_dir = dir.path().join("game");
    std::fs::create_dir_all(&game_dir).unwrap();
    let proc_root = empty_proc_root(dir.path());

    let entry = proc_root.join("777");
    std::fs::create_dir_all(&entry).unwrap();
    std::fs::write(
        entry.join("cmdline"),
        format!(
            "/usr/bin/python3\0tool.py\0--gameDir={}\0",
            game_dir.display()
        ),
    )
    .unwrap();

    assert!(check_in(&game_dir, &proc_root).running.is_none());
}

#[test]
fn process_scan_finds_the_client_main_class() {
    let dir = tempfile::tempdir().unwrap();
    let game_dir = dir.path().join("game");
    std::fs::create_dir_all(&game_dir).unwrap();
    let proc_root = empty_proc_root(dir.path());

    let entry = proc_root.join("31337");
    std::fs::create_dir_all(&entry).unwrap();
    std::fs::write(
        entry.join("cmdline"),
        "/usr/lib/jvm/java/bin/java\0-cp\0client.jar\0net.minecraft.client.main.Main\0--username\0Steve\0\0",
    )
    .unwrap();

    let running = check_in(&game_dir, &proc_root).running.expect("detected");
    assert_eq!(running.pid, Some(31337));
    assert!(running
        .command
        .unwrap()
        .contains("net.minecraft.client.main.Main"));
}

#[test]
fn missing_saves_and_proc_roots_report_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let game_dir = dir.path().join("game");
    std::fs::create_dir_all(&game_dir).unwrap();
    let proc_root = empty_proc_root(dir.path());

    let status = check_in(&game_dir, &proc_root);
    assert!(status.running.is_none());
    assert!(status.stale_locks.is_empty());
}

#[test]
fn an_unreadable_saves_path_blocks_as_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let game_dir = dir.path().join("game");
    std::fs::create_dir_all(&game_dir).unwrap();
    // A file named `saves` makes read_dir fail with NotADirectory on Unix; it
    // is not the normal "no saves directory" case, so it must block.
    let saves = game_dir.join("saves");
    std::fs::write(&saves, b"not a directory").unwrap();
    let proc_root = empty_proc_root(dir.path());

    let status = check_in(&game_dir, &proc_root);
    let running = status
        .running
        .expect("an unreadable saves path blocks as unknown");
    assert_eq!(running.kind, GameRunningKind::SessionLock);
    assert_eq!(running.lock_path.as_deref(), Some(saves.as_path()));
    assert!(
        running.detail.contains("saves") && running.detail.contains("could not be read"),
        "the detail names the path and the failure: {}",
        running.detail
    );
    assert_eq!(
        mine_utils_core::game_lock::ensure_writable(&game_dir)
            .unwrap_err()
            .code(),
        "game_running"
    );
}

fn write_proc_entry(proc_root: &Path, pid: i32, argv: &[String]) {
    let entry = proc_root.join(pid.to_string());
    std::fs::create_dir_all(&entry).unwrap();
    std::fs::write(entry.join("cmdline"), argv.join("\0") + "\0").unwrap();
}

fn set_proc_cwd(proc_root: &Path, pid: i32, cwd: &Path) {
    let entry = proc_root.join(pid.to_string());
    std::fs::create_dir_all(&entry).unwrap();
    std::os::unix::fs::symlink(cwd, entry.join("cwd")).unwrap();
}

#[test]
fn process_scan_finds_a_client_by_cwd_only() {
    let dir = tempfile::tempdir().unwrap();
    let game_dir = dir.path().join("game");
    std::fs::create_dir_all(&game_dir).unwrap();
    let proc_root = empty_proc_root(dir.path());

    // No vanilla main class and no --gameDir: only the cwd points at the game.
    write_proc_entry(
        &proc_root,
        6100,
        &[
            "/usr/lib/jvm/java/bin/java".to_string(),
            "-cp".to_string(),
            "client.jar".to_string(),
            "some.launcher.Wrapper".to_string(),
        ],
    );
    set_proc_cwd(&proc_root, 6100, &game_dir);

    let running = check_in(&game_dir, &proc_root)
        .running
        .expect("a java process whose cwd is the game dir is detected");
    assert_eq!(running.kind, GameRunningKind::Process);
    assert_eq!(running.pid, Some(6100));
}

#[test]
fn process_scan_finds_fabric_and_forge_wrappers() {
    for (pid, main_class) in [
        (6200, "net.fabricmc.loader.impl.launch.knot.KnotClient"),
        (6202, "cpw.mods.bootstraplauncher.BootstrapLauncher"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let game_dir = dir.path().join("game");
        std::fs::create_dir_all(&game_dir).unwrap();
        let proc_root = empty_proc_root(dir.path());
        write_proc_entry(
            &proc_root,
            pid,
            &[
                "/usr/bin/java".to_string(),
                "-cp".to_string(),
                "launcher.jar".to_string(),
                main_class.to_string(),
            ],
        );

        let running = check_in(&game_dir, &proc_root)
            .running
            .unwrap_or_else(|| panic!("{main_class} is detected"));
        assert_eq!(running.pid, Some(pid));
        assert!(running.command.unwrap().contains(main_class));
    }
}

#[test]
fn process_scan_matches_the_main_class_after_a_long_classpath() {
    let dir = tempfile::tempdir().unwrap();
    let game_dir = dir.path().join("game");
    std::fs::create_dir_all(&game_dir).unwrap();
    let proc_root = empty_proc_root(dir.path());

    // Real launchers put a multi-KB classpath before the main class. The
    // banner's command text truncates at 240 bytes, so the guard must match
    // the arguments themselves.
    let classpath = (0..120)
        .map(|index| format!("/lib/loader-{index}.jar"))
        .collect::<Vec<_>>()
        .join(":");
    assert!(classpath.len() > 2048, "the classpath is 2 KB or more");
    write_proc_entry(
        &proc_root,
        6250,
        &[
            "/usr/lib/jvm/java/bin/java".to_string(),
            "-cp".to_string(),
            classpath,
            "net.fabricmc.loader.impl.launch.knot.KnotClient".to_string(),
        ],
    );

    let running = check_in(&game_dir, &proc_root)
        .running
        .expect("the main class after a 2 KB classpath is detected");
    assert_eq!(running.pid, Some(6250));
    let command = running.command.expect("command text");
    assert!(
        command.ends_with('…'),
        "the display command is truncated: {command}"
    );
    assert!(
        !command.contains("KnotClient"),
        "the truncated display never shows the main class, proving the match used the args"
    );
}

#[test]
fn process_scan_ignores_a_bare_knot_wrapper() {
    let dir = tempfile::tempdir().unwrap();
    let game_dir = dir.path().join("game");
    std::fs::create_dir_all(&game_dir).unwrap();
    let proc_root = empty_proc_root(dir.path());

    // `Knot` is a substring of `KnotClient`; only the client entry point is a
    // running game, so a bare loader class must not block writes.
    write_proc_entry(
        &proc_root,
        6251,
        &[
            "/usr/bin/java".to_string(),
            "-cp".to_string(),
            "launcher.jar".to_string(),
            "net.fabricmc.loader.impl.launch.knot.Knot".to_string(),
        ],
    );

    assert!(check_in(&game_dir, &proc_root).running.is_none());
}

#[test]
fn process_scan_normalizes_trailing_slashes_and_canonical_paths() {
    for (pid, trailing) in [(6300, ""), (6301, "/")] {
        let dir = tempfile::tempdir().unwrap();
        let game_dir = dir.path().join("game");
        std::fs::create_dir_all(&game_dir).unwrap();
        let canonical = std::fs::canonicalize(&game_dir).unwrap();
        let proc_root = empty_proc_root(dir.path());

        write_proc_entry(
            &proc_root,
            pid,
            &[
                "/usr/bin/java".to_string(),
                "--gameDir".to_string(),
                format!("{}{trailing}", canonical.display()),
            ],
        );

        let running = check_in(&game_dir, &proc_root)
            .running
            .unwrap_or_else(|| panic!("pid {pid} with {trailing:?} must be detected"));
        assert_eq!(running.pid, Some(pid));
    }

    let dir = tempfile::tempdir().unwrap();
    let game_dir = dir.path().join("game");
    std::fs::create_dir_all(&game_dir).unwrap();
    let canonical = std::fs::canonicalize(&game_dir).unwrap();
    let proc_root = empty_proc_root(dir.path());
    write_proc_entry(
        &proc_root,
        6302,
        &[
            "/usr/bin/java".to_string(),
            format!("--gameDir={}/", canonical.display()),
        ],
    );
    let running = check_in(&game_dir, &proc_root)
        .running
        .expect("equals form");
    assert_eq!(running.pid, Some(6302));
}

#[test]
fn process_scan_resolves_a_relative_game_dir_against_proc_cwd() {
    let dir = tempfile::tempdir().unwrap();
    let game_dir = dir.path().join("game");
    std::fs::create_dir_all(&game_dir).unwrap();
    let proc_root = empty_proc_root(dir.path());

    // The process cwd is the parent and the argument is just `game`, so only
    // the relative resolution can match; the cwd check on its own cannot.
    write_proc_entry(
        &proc_root,
        6400,
        &[
            "/usr/bin/java".to_string(),
            "--gameDir".to_string(),
            "game".to_string(),
        ],
    );
    set_proc_cwd(&proc_root, 6400, dir.path());

    let running = check_in(&game_dir, &proc_root)
        .running
        .expect("a relative --gameDir resolves against the process cwd");
    assert_eq!(running.pid, Some(6400));
}

#[test]
fn a_different_cwd_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let game_dir = dir.path().join("game");
    let other = dir.path().join("other");
    std::fs::create_dir_all(&game_dir).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    let proc_root = empty_proc_root(dir.path());

    write_proc_entry(
        &proc_root,
        6500,
        &[
            "/usr/bin/java".to_string(),
            "-cp".to_string(),
            "client.jar".to_string(),
            "some.wrapper.Main".to_string(),
        ],
    );
    set_proc_cwd(&proc_root, 6500, &other);

    assert!(check_in(&game_dir, &proc_root).running.is_none());
}

#[test]
fn an_unprobeable_session_lock_blocks_as_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let game_dir = game_dir_with_world(dir.path());
    // A directory cannot be opened as a lock file; the probe must not call the
    // world stale just because it could not read session.lock.
    std::fs::create_dir_all(game_dir.join("saves/world/session.lock")).unwrap();
    let proc_root = empty_proc_root(dir.path());

    let status = check_in(&game_dir, &proc_root);
    let running = status
        .running
        .expect("an unprobeable session.lock blocks as unknown");
    assert_eq!(running.kind, GameRunningKind::SessionLock);
    assert!(status.stale_locks.is_empty());
    assert!(
        running.detail.contains("could not be probed"),
        "{}",
        running.detail
    );
    assert_eq!(
        running.lock_path.as_deref(),
        Some(game_dir.join("saves/world/session.lock").as_path())
    );
}

#[test]
fn a_long_non_ascii_cmdline_is_truncated_without_panicking() {
    let dir = tempfile::tempdir().unwrap();
    let game_dir = dir.path().join("game");
    std::fs::create_dir_all(&game_dir).unwrap();
    let proc_root = empty_proc_root(dir.path());
    let noisy = "é".repeat(400);
    write_proc_entry(
        &proc_root,
        6600,
        &[
            "/usr/bin/java".to_string(),
            "-cp".to_string(),
            "net.minecraft.client.main.Main".to_string(),
            noisy,
        ],
    );

    let running = check_in(&game_dir, &proc_root).running.expect("detected");
    let command = running.command.expect("command text");
    assert!(
        command.ends_with('…'),
        "the command is truncated with an ellipsis: {command}"
    );
    assert!(
        command.chars().count() <= 241,
        "truncation happens on a char boundary"
    );
}

#[test]
fn every_world_is_scanned_before_a_held_lock_wins() {
    if !lock_detection_supported() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let game_dir = game_dir_with_world(dir.path());
    let held_path = game_dir.join("saves/world/session.lock");
    std::fs::write(&held_path, b"\x00").unwrap();
    let free_world = game_dir.join("saves/other");
    std::fs::create_dir_all(&free_world).unwrap();
    let free_path = free_world.join("session.lock");
    std::fs::write(&free_path, b"\x00").unwrap();
    let proc_root = empty_proc_root(dir.path());

    let mut child = spawn_lock_holder(&held_path);
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut status = check_in(&game_dir, &proc_root);
    while status.running.is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        status = check_in(&game_dir, &proc_root);
    }
    let _ = child.kill();
    let _ = child.wait();

    assert!(status.running.is_some(), "the held lock is found");
    assert_eq!(
        status.stale_locks,
        vec![free_path],
        "the free lock is reported too, not hidden by the early return"
    );
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
    fn move_to_trash(&self, path: &Path) -> Result<(), mine_utils_core::pack::PackError> {
        self.calls.lock().unwrap().push(path.to_path_buf());
        Ok(())
    }
}

fn stack_paths(game_dir: &Path, data_dir: &Path) -> StackPaths {
    std::fs::create_dir_all(game_dir.join("resourcepacks")).unwrap();
    StackPaths::for_game_dir(game_dir, data_dir, BackupsConfig::default())
}

/// A guard that returns `Ok` for every call except `fail_at`, so a test can
/// fail exactly the post-backup or pre-trash re-check.
struct ScriptedGuard {
    calls: std::cell::Cell<usize>,
    fail_at: usize,
}

impl ScriptedGuard {
    fn failing_at(fail_at: usize) -> Self {
        ScriptedGuard {
            calls: std::cell::Cell::new(0),
            fail_at,
        }
    }

    fn calls(&self) -> usize {
        self.calls.get()
    }
}

impl WriteGuard for ScriptedGuard {
    fn ensure_writable(&self, _game_dir: &Path) -> Result<(), AppError> {
        let calls = self.calls.get() + 1;
        self.calls.set(calls);
        if calls == self.fail_at {
            return Err(AppError::GameRunning {
                running: GameRunning {
                    kind: GameRunningKind::Process,
                    detail: "the game started mid-write".to_string(),
                    lock_path: None,
                    pid: Some(4242),
                    command: None,
                },
            });
        }
        Ok(())
    }
}

#[test]
fn a_guard_failure_after_the_backup_keeps_the_old_bytes_and_the_backup() {
    let dir = tempfile::tempdir().unwrap();
    let game_dir = game_dir_with_world(dir.path());
    let paths = stack_paths(&game_dir, &dir.path().join("data"));
    let original = b"resourcePacks:[\"vanilla\"]\n";
    std::fs::write(paths.options_path(), original).unwrap();

    // First check passes; the re-check after the backup fails.
    let guard = ScriptedGuard::failing_at(2);
    let err = resource_stack::set_enabled_with_guard(&paths, "a.zip", true, &guard).unwrap_err();

    assert_eq!(err.code(), "game_running");
    assert_eq!(guard.calls(), 2, "the post-backup guard ran");
    assert_eq!(
        std::fs::read(paths.options_path()).unwrap(),
        original,
        "the old bytes survive a guard failure before the replace"
    );
    let backups = backup::list(paths.backups_root()).unwrap();
    assert_eq!(backups.len(), 1, "the previous bytes became a backup");
    assert_eq!(
        std::fs::read(paths.backups_root().join(&backups[0].file_name)).unwrap(),
        original
    );
}

#[test]
fn a_guard_failure_before_the_trash_cleans_references_but_keeps_the_pack() {
    let dir = tempfile::tempdir().unwrap();
    let game_dir = game_dir_with_world(dir.path());
    let paths = stack_paths(&game_dir, &dir.path().join("data"));
    let original = b"resourcePacks:[\"vanilla\",\"file/Gone.zip\"]\n";
    std::fs::write(paths.options_path(), original).unwrap();
    let pack = game_dir.join("resourcepacks/Gone.zip");
    std::fs::write(&pack, b"zip").unwrap();

    // The first outer check passes; the second, immediately before the trash,
    // fails. The cleanup write uses the real guard, which passes in this idle
    // test directory, so the references are already cleaned.
    let guard = ScriptedGuard::failing_at(2);
    let trash = RecordingTrash::new();
    let err =
        resource_stack::delete_pack_safely_with_guard(&paths, &pack, &trash, &guard).unwrap_err();

    assert_eq!(err.code(), "game_running");
    assert_eq!(guard.calls(), 2, "the pre-trash guard ran");
    assert!(pack.is_file(), "the pack is untouched");
    assert!(trash.calls().is_empty(), "nothing was trashed");
    let text = std::fs::read_to_string(paths.options_path()).unwrap();
    assert!(
        !text.contains("file/Gone.zip"),
        "the references were cleaned before the final check: {text}"
    );
    assert!(text.contains("vanilla"));
    assert_eq!(
        backup::list(paths.backups_root()).unwrap().len(),
        1,
        "the cleanup write left a backup"
    );
}

#[test]
fn a_held_lock_blocks_set_enabled_and_leaves_options_and_backups() {
    if !lock_detection_supported() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let game_dir = game_dir_with_world(dir.path());
    let paths = stack_paths(&game_dir, &dir.path().join("data"));
    let original = b"resourcePacks:[\"vanilla\"]\n";
    std::fs::write(paths.options_path(), original).unwrap();
    let lock_path = game_dir.join("saves/world/session.lock");
    std::fs::write(&lock_path, b"\x00").unwrap();

    let mut child = spawn_lock_holder(&lock_path);
    let running = wait_for_running(&game_dir, Path::new("/proc"));
    let err = resource_stack::set_enabled(&paths, "a.zip", true).unwrap_err();
    let _ = child.kill();
    let _ = child.wait();

    assert!(running.is_some(), "the helper really holds the lock");
    assert_eq!(err.code(), "game_running");
    assert_eq!(std::fs::read(paths.options_path()).unwrap(), original);
    assert!(backup::list(paths.backups_root()).unwrap().is_empty());
}

#[test]
fn a_held_lock_blocks_restore_and_leaves_options_and_backups() {
    if !lock_detection_supported() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let game_dir = game_dir_with_world(dir.path());
    let paths = stack_paths(&game_dir, &dir.path().join("data"));
    let current = b"resourcePacks:[\"current\"]\n";
    std::fs::write(paths.options_path(), current).unwrap();
    let created = backup::create(
        paths.backups_root(),
        b"resourcePacks:[\"old\"]\n",
        paths.backups(),
    )
    .unwrap()
    .created
    .unwrap();
    let lock_path = game_dir.join("saves/world/session.lock");
    std::fs::write(&lock_path, b"\x00").unwrap();

    let mut child = spawn_lock_holder(&lock_path);
    let running = wait_for_running(&game_dir, Path::new("/proc"));
    let err = resource_stack::restore_backup(&paths, &created.file_name).unwrap_err();
    let _ = child.kill();
    let _ = child.wait();

    assert!(running.is_some());
    assert_eq!(err.code(), "game_running");
    assert_eq!(std::fs::read(paths.options_path()).unwrap(), current);
    assert_eq!(
        backup::list(paths.backups_root()).unwrap().len(),
        1,
        "no pre-restore backup was written"
    );
}

#[test]
fn a_held_lock_blocks_the_guarded_delete() {
    if !lock_detection_supported() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let game_dir = game_dir_with_world(dir.path());
    let paths = stack_paths(&game_dir, &dir.path().join("data"));
    let original = b"resourcePacks:[\"vanilla\",\"file/Gone.zip\"]\n";
    std::fs::write(paths.options_path(), original).unwrap();
    let pack = game_dir.join("resourcepacks/Gone.zip");
    std::fs::write(&pack, b"zip").unwrap();
    let lock_path = game_dir.join("saves/world/session.lock");
    std::fs::write(&lock_path, b"\x00").unwrap();

    let trash = RecordingTrash::new();
    let mut child = spawn_lock_holder(&lock_path);
    let running = wait_for_running(&game_dir, Path::new("/proc"));
    let err = resource_stack::delete_pack_safely(&paths, &pack, &trash).unwrap_err();
    let _ = child.kill();
    let _ = child.wait();

    assert!(running.is_some());
    assert_eq!(err.code(), "game_running");
    assert!(pack.is_file(), "the pack is untouched");
    assert_eq!(std::fs::read(paths.options_path()).unwrap(), original);
    assert!(trash.calls().is_empty(), "nothing was trashed");
}

#[test]
fn process_probe_reports_unavailable_and_empty_roots() {
    let root = tempfile::tempdir().unwrap();
    let game = root.path().join("game");
    let empty = root.path().join("empty");
    std::fs::create_dir_all(&game).unwrap();
    std::fs::create_dir_all(&empty).unwrap();
    let unavailable = serde_json::to_value(check_in(&game, &root.path().join("missing"))).unwrap();
    let idle = serde_json::to_value(check_in(&game, &empty)).unwrap();
    assert_eq!(unavailable["process_probe_available"], false);
    assert_eq!(unavailable["running"], serde_json::Value::Null);
    assert_eq!(idle["process_probe_available"], true);
    assert_eq!(idle["running"], serde_json::Value::Null);
}

#[test]
fn a_mode_000_proc_root_marks_the_probe_unavailable() {
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let game = root.path().join("game");
    let proc_root = root.path().join("proc");
    std::fs::create_dir_all(&game).unwrap();
    std::fs::create_dir_all(&proc_root).unwrap();
    std::fs::write(proc_root.join("4242"), b"not-a-directory").unwrap();
    let mut blocked = std::fs::metadata(&proc_root).unwrap().permissions();
    blocked.set_mode(0o000);
    std::fs::set_permissions(&proc_root, blocked).unwrap();

    let status = serde_json::to_value(check_in(&game, &proc_root)).unwrap();
    std::fs::set_permissions(&proc_root, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(status["process_probe_available"], false);
    assert_eq!(status["running"], serde_json::Value::Null);
}

#[test]
fn a_held_lock_does_not_mark_the_unrun_process_probe_unavailable() {
    if !lock_detection_supported() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let game_dir = game_dir_with_world(dir.path());
    let lock_path = game_dir.join("saves/world/session.lock");
    std::fs::write(&lock_path, b"\x00").unwrap();
    let missing_proc = dir.path().join("missing-proc");

    let mut child = spawn_lock_holder(&lock_path);
    let status = wait_for_status(&game_dir, &missing_proc);
    let _ = child.kill();
    let _ = child.wait();

    let status = status.expect("the held session.lock is detected");
    assert_eq!(
        status.running.as_ref().map(|running| running.kind),
        Some(GameRunningKind::SessionLock)
    );
    assert!(status.process_probe_available);
}
