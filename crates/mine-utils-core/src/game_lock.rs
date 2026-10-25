//! Detect a running Minecraft client before any `options.txt` write.
//!
//! Two independent checks: every `saves/*/session.lock` is probed with a POSIX
//! record lock (`fcntl(F_GETLK)`), because that is what Java takes, and
//! `/proc/*/cmdline` plus `/proc/*/cwd` are scanned for a `java` process that
//! runs a known client main class or points at the active game dir. Either
//! check blocks writes. A `session.lock` nobody holds is stale and is reported
//! without blocking; one that cannot be probed is unknown and blocks.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Main classes that mean a Minecraft client is running. Modded launchers wrap
/// the vanilla class, so the wrapper name is what their command line carries.
const CLIENT_MAIN_CLASSES: &[&str] = &[
    "net.minecraft.client.main.Main",
    "net.fabricmc.loader.impl.launch.knot.KnotClient",
    "cpw.mods.bootstraplauncher.BootstrapLauncher",
];

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

/// What `F_GETLK` reported. A probe failure is not evidence that the file is
/// free, so it is carried as `Unknown` and blocks the write.
#[cfg(unix)]
enum LockProbe {
    Free,
    Held(i32),
    Unknown(String),
}

/// Probe every `saves/*/session.lock` before deciding. Minecraft's SessionLock
/// takes a POSIX write lock over the whole file, so `F_GETLK` names the holder.
/// A lock file with no active lock is stale. An unprobeable file is unknown and
/// blocks, because treating it as stale could overwrite a live game's options.
#[cfg(unix)]
fn session_lock_check(game_dir: &Path, stale: &mut Vec<PathBuf>) -> Option<GameRunning> {
    let saves = game_dir.join("saves");
    let mut worlds: Vec<PathBuf> = match std::fs::read_dir(&saves) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect(),
        // A missing saves directory is normal: no world has ever locked a file.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
        // Any other failure means the locks cannot be inspected, which is
        // unknown, not evidence that the game is closed.
        Err(err) => {
            return Some(GameRunning {
                kind: GameRunningKind::SessionLock,
                detail: format!("{} could not be read: {err}", saves.display()),
                lock_path: Some(saves),
                pid: None,
                command: None,
            });
        }
    };
    worlds.sort();

    let mut held = None;
    let mut unknown = None;
    for world in worlds {
        let lock_path = world.join("session.lock");
        let file = match open_lock_file(&lock_path) {
            Ok(Some(file)) => file,
            Ok(None) => continue,
            Err(reason) => {
                unknown.get_or_insert_with(|| lock_unknown(&lock_path, reason));
                continue;
            }
        };
        match posix_lock_holder(&file) {
            LockProbe::Held(pid) => {
                held.get_or_insert_with(|| GameRunning {
                    kind: GameRunningKind::SessionLock,
                    detail: format!("{} is locked by pid {pid}", lock_path.display()),
                    lock_path: Some(lock_path),
                    pid: Some(pid),
                    command: None,
                });
            }
            LockProbe::Free => stale.push(lock_path),
            LockProbe::Unknown(reason) => {
                unknown.get_or_insert_with(|| lock_unknown(&lock_path, reason));
            }
        }
    }
    // Every lock is probed before this choice, so a held lock never hides an
    // unknown one and the stale list is always complete.
    held.or(unknown)
}

#[cfg(unix)]
fn lock_unknown(path: &Path, reason: String) -> GameRunning {
    GameRunning {
        kind: GameRunningKind::SessionLock,
        detail: format!("{} could not be probed: {reason}", path.display()),
        lock_path: Some(path.to_path_buf()),
        pid: None,
        command: None,
    }
}

/// Open a lock file for probing. `Ok(None)` means the path is absent, which is
/// normal. An existing but unopenable path is an error, never silently stale.
#[cfg(unix)]
fn open_lock_file(path: &Path) -> Result<Option<std::fs::File>, String> {
    use std::os::unix::fs::OpenOptionsExt;
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("could not read metadata: {err}")),
    };
    if !metadata.is_file() {
        return Err("session.lock is not a regular file".to_string());
    }
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .mode(0o644)
        .open(path)
        .or_else(|_| std::fs::File::open(path))
        .map(Some)
        .map_err(|err| format!("could not open the lock file: {err}"))
}

/// Ask the kernel who holds a write lock on this file. Locks are per process,
/// so the caller must probe from outside the holder.
#[cfg(unix)]
fn posix_lock_holder(file: &std::fs::File) -> LockProbe {
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
        return LockProbe::Unknown(format!(
            "fcntl(F_GETLK) failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    if request.l_type == libc::F_UNLCK as libc::c_short {
        return LockProbe::Free;
    }
    if request.l_pid > 0 {
        LockProbe::Held(request.l_pid)
    } else {
        LockProbe::Unknown("the kernel reported a lock without a holder pid".to_string())
    }
}

#[cfg(not(unix))]
fn session_lock_check(_game_dir: &Path, _stale: &mut Vec<PathBuf>) -> Option<GameRunning> {
    None
}

/// Scan `/proc/*/cmdline` for a `java` process whose command line contains a
/// known client main class, a `--gameDir` pointing at the active game dir, or a
/// working directory equal to the active game dir. The cwd check covers
/// launchers that set the process directory instead of passing `--gameDir`.
fn process_check(game_dir: &Path, proc_root: &Path) -> Option<GameRunning> {
    let canonical_game_dir = std::fs::canonicalize(game_dir).ok();
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
        // Match every argument, untruncated: a real launcher puts a multi-KB
        // classpath before the main class, so the display string is too short.
        let named_main_class = args.iter().any(|arg| {
            CLIENT_MAIN_CLASSES
                .iter()
                .any(|main_class| arg.contains(main_class))
        });
        let cwd = std::fs::read_link(proc_dir.join("cwd")).ok();
        if !(named_main_class
            || game_dir_matches(
                &args,
                game_dir,
                canonical_game_dir.as_deref(),
                cwd.as_deref(),
            )
            || cwd_matches(cwd.as_deref(), game_dir, canonical_game_dir.as_deref()))
        {
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

/// A short display form for the banner. Guard decisions must inspect `args`
/// themselves; real launchers put a multi-KB classpath before the main class.
fn cmdline_text(args: &[String]) -> String {
    let mut command = args.join(" ");
    if command.len() > 240 {
        // Truncate on a char boundary so a non-ASCII argument cannot panic.
        let mut end = 240;
        while end > 0 && !command.is_char_boundary(end) {
            end -= 1;
        }
        command.truncate(end);
        command.push('…');
    }
    command
}

fn game_dir_matches(
    args: &[String],
    game_dir: &Path,
    canonical_game_dir: Option<&Path>,
    cwd: Option<&Path>,
) -> bool {
    args.iter().enumerate().any(|(index, arg)| {
        let value = if arg == "--gameDir" {
            args.get(index + 1).map(String::as_str)
        } else {
            arg.strip_prefix("--gameDir=")
        };
        value.is_some_and(|value| game_dir_value_matches(value, game_dir, canonical_game_dir, cwd))
    })
}

/// `--gameDir` values may be absolute, relative, or carry trailing separators.
/// Relative values are resolved against the process cwd when `/proc` exposes it.
fn game_dir_value_matches(
    value: &str,
    game_dir: &Path,
    canonical_game_dir: Option<&Path>,
    cwd: Option<&Path>,
) -> bool {
    let trimmed = value.trim_end_matches(['/', '\\']);
    let trimmed = if trimmed.is_empty() { value } else { trimmed };
    let candidate = Path::new(trimmed);
    let resolved = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else if let Some(cwd) = cwd {
        cwd.join(candidate)
    } else {
        candidate.to_path_buf()
    };
    if resolved == game_dir {
        return true;
    }
    // A relative value without a proc cwd cannot be resolved against the
    // client's directory; do not resolve it against Mine Utils' own cwd.
    if !candidate.is_absolute() && cwd.is_none() {
        return false;
    }
    match (std::fs::canonicalize(&resolved), canonical_game_dir) {
        (Ok(resolved), Some(target)) => resolved == target,
        _ => false,
    }
}

fn cwd_matches(cwd: Option<&Path>, game_dir: &Path, canonical_game_dir: Option<&Path>) -> bool {
    let Some(cwd) = cwd else {
        return false;
    };
    if cwd == game_dir {
        return true;
    }
    match (std::fs::canonicalize(cwd), canonical_game_dir) {
        (Ok(cwd), Some(target)) => cwd == target,
        _ => false,
    }
}
