//! Game guard tests: a real second process holds a POSIX write lock, and a
//! fake `/proc` proves the process scan.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use mine_utils_core::backup;
use mine_utils_core::game_lock::{check_in, lock_detection_supported, GameRunningKind};
use mine_utils_core::pack_scan::TrashService;
use mine_utils_core::resource_stack::{self, StackPaths};
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
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Some(running) = check_in(game_dir, proc_root).running {
            return Some(running);
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
        (6201, "net.fabricmc.loader.impl.launch.knot.Knot"),
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

    write_proc_entry(
        &proc_root,
        6400,
        &[
            "/usr/bin/java".to_string(),
            "--gameDir".to_string(),
            ".".to_string(),
        ],
    );
    set_proc_cwd(&proc_root, 6400, &game_dir);

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

#[test]
fn a_held_lock_blocks_set_enabled_and_leaves_options_and_backups() {
    if !lock_detection_supported() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let game_dir = game_dir_with_world(dir.path());
    let paths = stack_paths(&game_dir, &dir.path().join("data"));
    let original = b"resourcePacks:[\"vanilla\"]\n";
    std::fs::write(&paths.options_path, original).unwrap();
    let lock_path = game_dir.join("saves/world/session.lock");
    std::fs::write(&lock_path, b"\x00").unwrap();

    let mut child = spawn_lock_holder(&lock_path);
    let running = wait_for_running(&game_dir, Path::new("/proc"));
    let err = resource_stack::set_enabled(&paths, "a.zip", true).unwrap_err();
    let _ = child.kill();
    let _ = child.wait();

    assert!(running.is_some(), "the helper really holds the lock");
    assert_eq!(err.code(), "game_running");
    assert_eq!(std::fs::read(&paths.options_path).unwrap(), original);
    assert!(backup::list(&paths.backups_root).unwrap().is_empty());
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
    std::fs::write(&paths.options_path, current).unwrap();
    let created = backup::create(
        &paths.backups_root,
        b"resourcePacks:[\"old\"]\n",
        &paths.backups,
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
    assert_eq!(std::fs::read(&paths.options_path).unwrap(), current);
    assert_eq!(
        backup::list(&paths.backups_root).unwrap().len(),
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
    std::fs::write(&paths.options_path, original).unwrap();
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
    assert_eq!(std::fs::read(&paths.options_path).unwrap(), original);
    assert!(trash.calls().is_empty(), "nothing was trashed");
}
