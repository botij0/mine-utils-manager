//! Detect a running Minecraft client before any `options.txt` write.
//!
//! Two independent checks: every `saves/*/session.lock` is probed with a POSIX
//! record lock (`fcntl(F_GETLK)`), because that is what Java takes, and
//! `/proc/*/cmdline` is scanned for a `java` process that runs the Minecraft
//! client or points at the active game dir. Either check blocks writes. A
//! `session.lock` file nobody holds is stale and is reported without blocking.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How the running client was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameRunningKind {
    SessionLock,
    Process,
}

/// Enough detail for the banner to name the lock path or the process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameRunning {
    pub kind: GameRunningKind,
    pub detail: String,
    pub lock_path: Option<PathBuf>,
    pub pid: Option<i32>,
    pub command: Option<String>,
}

/// The full running-game state for one game dir.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameStatus {
    pub running: Option<GameRunning>,
    /// `session.lock` files that exist but hold no lock. Stale files never
    /// block a write; they are surfaced so the UI can explain that.
    pub stale_locks: Vec<PathBuf>,
    /// False when the platform cannot probe POSIX locks. The fallback reports
    /// unsupported instead of pretending the game is closed.
    pub supported: bool,
}

impl GameStatus {
    fn idle(supported: bool) -> GameStatus {
        GameStatus {
            running: None,
            stale_locks: Vec::new(),
            supported,
        }
    }
}

/// Whether lock detection works on this platform. The POSIX probe is Unix-only.
pub fn lock_detection_supported() -> bool {
    cfg!(unix)
}

/// Check the active game dir for a running client.
pub fn check(game_dir: &Path) -> GameStatus {
    check_in(game_dir, Path::new("/proc"))
}

/// Check with an injectable proc root so tests can fake process listings.
pub fn check_in(game_dir: &Path, proc_root: &Path) -> GameStatus {
    let mut status = GameStatus::idle(lock_detection_supported());
    if !status.supported {
        return status;
    }

    if let Some(running) = session_lock_check(game_dir, &mut status.stale_locks) {
        status.running = Some(running);
        return status;
    }
    if let Some(running) = process_check(game_dir, proc_root) {
        status.running = Some(running);
    }
    status
}

/// Block a write when either check fires.
pub fn ensure_writable(game_dir: &Path) -> Result<(), crate::error::AppError> {
    let status = check(game_dir);
    match status.running {
        Some(running) => Err(crate::error::AppError::GameRunning { running }),
        None => Ok(()),
    }
}

/// Probe every `saves/*/session.lock`. Minecraft's SessionLock takes a POSIX
/// write lock over the whole file, so `F_GETLK` names the holder. A lock file
/// with no active lock is stale and does not block.
#[cfg(unix)]
fn session_lock_check(game_dir: &Path, stale: &mut Vec<PathBuf>) -> Option<GameRunning> {
    let saves = game_dir.join("saves");
    let mut worlds: Vec<PathBuf> = match std::fs::read_dir(&saves) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect(),
        Err(_) => return None,
    };
    worlds.sort();

    for world in worlds {
        let lock_path = world.join("session.lock");
        let file = match open_lock_file(&lock_path) {
            Some(file) => file,
            None => continue,
        };
        match posix_lock_holder(&file) {
            Some(pid) => {
                return Some(GameRunning {
                    kind: GameRunningKind::SessionLock,
                    detail: format!("{} is locked by pid {pid}", lock_path.display()),
                    lock_path: Some(lock_path),
                    pid: Some(pid),
                    command: None,
                });
            }
            None => stale.push(lock_path),
        }
    }
    None
}

#[cfg(unix)]
fn open_lock_file(path: &Path) -> Option<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .mode(0o644)
        .open(path)
        .or_else(|_| std::fs::File::open(path))
        .ok()
}

/// Ask the kernel who holds a write lock on this file. Returns the holder's
/// pid, or None when the file is free (or the probe itself failed). Locks are
/// per process, so the caller must probe from outside the holder.
#[cfg(unix)]
fn posix_lock_holder(file: &std::fs::File) -> Option<i32> {
    use std::os::unix::io::AsRawFd;

    let mut request = libc::flock {
        l_type: libc::F_WRLCK as libc::c_short,
        l_whence: libc::SEEK_SET as libc::c_short,
        l_start: 0,
        l_len: 0,
        l_pid: 0,
    };
    let result = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETLK, &mut request) };
    if result == -1 {
        return None;
    }
    if request.l_type == libc::F_UNLCK as libc::c_short {
        None
    } else {
        Some(request.l_pid)
    }
}

#[cfg(not(unix))]
fn session_lock_check(_game_dir: &Path, _stale: &mut Vec<PathBuf>) -> Option<GameRunning> {
    None
}

/// Scan `/proc/*/cmdline` for a `java` process whose command line contains the
/// Minecraft client main class or a `--gameDir` pointing at the active game
/// dir, in either `--gameDir <path>` or `--gameDir=<path>` form.
fn process_check(game_dir: &Path, proc_root: &Path) -> Option<GameRunning> {
    let mut pids: Vec<PathBuf> = match std::fs::read_dir(proc_root) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.chars().all(|c| c.is_ascii_digit()))
            })
            .collect(),
        Err(_) => return None,
    };
    pids.sort();

    for proc_dir in pids {
        let cmdline = match std::fs::read(proc_dir.join("cmdline")) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        let args: Vec<String> = cmdline
            .split(|byte| *byte == 0)
            .filter(|part| !part.is_empty())
            .map(|part| String::from_utf8_lossy(part).into_owned())
            .collect();
        if args.is_empty() {
            continue;
        }
        let program = Path::new(&args[0])
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_string();
        if !program.starts_with("java") {
            continue;
        }
        let command = cmdline_text(&args);
        let named_main_class = command.contains("net.minecraft.client.main.Main");
        if !(named_main_class || game_dir_matches(&args, game_dir)) {
            continue;
        }

        let pid = proc_dir
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.parse::<i32>().ok());
        return Some(GameRunning {
            kind: GameRunningKind::Process,
            detail: match pid {
                Some(pid) => format!("pid {pid} runs {program}"),
                None => format!("{program} is running"),
            },
            lock_path: None,
            pid,
            command: Some(command),
        });
    }
    None
}

fn cmdline_text(args: &[String]) -> String {
    let mut command = args.join(" ");
    if command.len() > 240 {
        command.truncate(240);
        command.push('…');
    }
    command
}

fn game_dir_matches(args: &[String], game_dir: &Path) -> bool {
    let target = game_dir.to_string_lossy();
    let canonical = std::fs::canonicalize(game_dir)
        .map(|path| path.to_string_lossy().into_owned())
        .ok();
    args.iter().enumerate().any(|(index, arg)| {
        if arg == "--gameDir" {
            return args
                .get(index + 1)
                .is_some_and(|value| value == &target || canonical.as_deref() == Some(value));
        }
        if let Some(value) = arg.strip_prefix("--gameDir=") {
            return value == target || canonical.as_deref() == Some(value);
        }
        false
    })
}
