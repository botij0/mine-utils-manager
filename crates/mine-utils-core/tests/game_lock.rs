//! Game guard tests: a real second process holds a POSIX write lock, and a
//! fake `/proc` proves the process scan.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use mine_utils_core::game_lock::{check_in, lock_detection_supported, GameRunningKind};

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
