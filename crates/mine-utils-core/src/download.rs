//! Verified pack downloads from the Modrinth CDN.
//!
//! A file streams into a unique temp file inside the target directory, never
//! held fully in memory. The SHA-512 from the version metadata is checked
//! while streaming, the zip root (`assets` for resource packs, `data` for
//! datapacks) is validated, and only then may the caller commit the temp
//! into place. The temp is deleted on every failure.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicU8, AtomicUsize, Ordering};

use sha2::{Digest, Sha256, Sha512};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use crate::fs_commit::atomic_no_replace;
use crate::modrinth::{ModrinthClient, Transport, MAX_DOWNLOAD_BYTES};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

const MAX_READER_WORKERS: usize = 8;
static READER_WORKERS: AtomicUsize = AtomicUsize::new(0);

struct ReaderPermit;

impl ReaderPermit {
    fn acquire(control: &InstallControl) -> Result<Self, DownloadError> {
        loop {
            control.check()?;
            let live = READER_WORKERS.load(Ordering::SeqCst);
            if live < MAX_READER_WORKERS
                && READER_WORKERS
                    .compare_exchange(live, live + 1, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
            {
                return Ok(Self);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for ReaderPermit {
    fn drop(&mut self) {
        READER_WORKERS.fetch_sub(1, Ordering::SeqCst);
    }
}

const DOWNLOADING: u8 = 0;
const CANCELLED: u8 = 1;
const COMMITTING: u8 = 2;

#[derive(Debug, Clone)]
pub struct InstallControl {
    state: Arc<AtomicU8>,
    deadline: Instant,
}

impl Default for InstallControl {
    fn default() -> Self {
        Self::with_timeout(Duration::from_secs(crate::modrinth::DOWNLOAD_TIMEOUT_SECS))
    }
}

impl InstallControl {
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            state: Arc::new(AtomicU8::new(DOWNLOADING)),
            deadline: Instant::now() + timeout,
        }
    }

    pub fn cancel(&self) -> bool {
        self.state
            .compare_exchange(DOWNLOADING, CANCELLED, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
            || self.state.load(Ordering::SeqCst) == CANCELLED
    }

    pub fn check(&self) -> Result<(), DownloadError> {
        if self.state.load(Ordering::SeqCst) == CANCELLED {
            return Err(DownloadError::Cancelled);
        }
        if Instant::now() >= self.deadline {
            return Err(DownloadError::Timeout);
        }
        Ok(())
    }

    /// Cancellation and the write phase race on this transition. After it
    /// succeeds, finish must complete import, enablement, and provenance.
    pub fn begin_commit(&self) -> Result<(), DownloadError> {
        self.check()?;
        self.state
            .compare_exchange(DOWNLOADING, COMMITTING, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| ())
            .map_err(|_| DownloadError::Cancelled)
    }
}

/// What the download verifier reports. Codes are the wire codes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadError {
    Cancelled,
    Timeout,
    /// The CDN could not be reached. The UI shows offline.
    Offline {
        reason: String,
    },
    /// The CDN answered with a non-2xx status after retries.
    Status {
        status: u16,
        url: String,
    },
    /// A redirect left the Modrinth hosts. Refused, not retried.
    BadHost {
        host: String,
    },
    /// More than 500 MB arrived. The temp is deleted, nothing is kept.
    TooLarge {
        url: String,
    },
    /// The stream failed or the target dir was unusable.
    Io {
        reason: String,
    },
    /// The bytes do not match the version metadata. Not retried.
    HashMismatch {
        expected: String,
        actual: String,
    },
    /// The bytes are not a zip. Not retried.
    NotZip {
        reason: String,
    },
    /// The zip has no `assets` (resource) or `data` (datapack) root.
    WrongRoot {
        expected: String,
        found: Vec<String>,
    },
    /// The file name is unsafe for the target directory.
    BadFilename {
        name: String,
    },
    /// The destination already exists; the installer routes this through the
    /// duplicate/collision prompt instead of overwriting.
    Exists {
        name: String,
    },
}

impl DownloadError {
    pub fn code(&self) -> &'static str {
        self.error_code().as_str()
    }

    pub fn error_code(&self) -> crate::ErrorCode {
        match self {
            DownloadError::Cancelled => crate::ErrorCode::DownloadCancelled,
            DownloadError::Timeout => crate::ErrorCode::DownloadTimeout,
            DownloadError::Offline { .. } => crate::ErrorCode::DownloadOffline,
            DownloadError::Status { .. } => crate::ErrorCode::DownloadStatus,
            DownloadError::BadHost { .. } => crate::ErrorCode::DownloadHost,
            DownloadError::TooLarge { .. } => crate::ErrorCode::DownloadTooLarge,
            DownloadError::Io { .. } => crate::ErrorCode::DownloadIo,
            DownloadError::HashMismatch { .. } => crate::ErrorCode::DownloadHashMismatch,
            DownloadError::NotZip { .. } => crate::ErrorCode::DownloadNotZip,
            DownloadError::WrongRoot { .. } => crate::ErrorCode::DownloadWrongRoot,
            DownloadError::BadFilename { .. } => crate::ErrorCode::DownloadFilename,
            DownloadError::Exists { .. } => crate::ErrorCode::DownloadExists,
        }
    }
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DownloadError::Cancelled => write!(f, "Install cancelled."),
            DownloadError::Timeout => write!(f, "Download exceeded its total deadline. Try again."),
            DownloadError::Offline { reason } => {
                write!(f, "download failed: CDN unreachable ({reason})")
            }
            DownloadError::Status { status, url } => {
                write!(f, "download failed: status {status} for {url}")
            }
            DownloadError::BadHost { host } => {
                write!(f, "download refused: redirect to unexpected host {host}")
            }
            DownloadError::TooLarge { url } => {
                write!(f, "download refused: {url} exceeds the 500 MB cap")
            }
            DownloadError::Io { reason } => write!(f, "download failed: {reason}"),
            DownloadError::HashMismatch { expected, actual } => {
                write!(
                    f,
                    "download failed: SHA-512 mismatch (expected {expected}, got {actual})"
                )
            }
            DownloadError::NotZip { reason } => {
                write!(f, "download failed: not a zip archive ({reason})")
            }
            DownloadError::WrongRoot { expected, found } => {
                write!(
                    f,
                    "download failed: zip has no `{expected}/` root (saw {})",
                    summarize_roots(found)
                )
            }
            DownloadError::BadFilename { name } => {
                write!(f, "download refused: unsafe file name {name:?}")
            }
            DownloadError::Exists { name } => {
                write!(f, "download stopped: {name} already exists")
            }
        }
    }
}

fn summarize_roots(found: &[String]) -> String {
    if found.is_empty() {
        return "an empty archive".to_string();
    }
    let mut roots: Vec<&str> = found
        .iter()
        .map(|name| name.split('/').next().unwrap_or(name))
        .collect();
    roots.sort();
    roots.dedup();
    roots.truncate(5);
    roots.join(", ")
}

impl std::error::Error for DownloadError {}

impl From<crate::modrinth::ModrinthError> for DownloadError {
    fn from(err: crate::modrinth::ModrinthError) -> Self {
        use crate::modrinth::ModrinthError as E;
        match err {
            E::Offline { reason } => DownloadError::Offline { reason },
            E::Status { status, url } | E::RetryExhausted { status, url } => {
                DownloadError::Status { status, url }
            }
            E::Redirect { host }
            | E::TooManyRedirects { url: host }
            | E::BadUrl { url: host, .. } => DownloadError::BadHost { host },
            E::BodyTooLarge { url } => DownloadError::TooLarge { url },
            E::Parse { reason } => DownloadError::Io { reason },
        }
    }
}

/// Which archive root the downloaded zip must contain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZipRoot {
    /// Resource packs carry `assets/`.
    Assets,
    /// Datapacks carry `data/`.
    Data,
}

impl ZipRoot {
    fn dir(self) -> &'static str {
        match self {
            ZipRoot::Assets => "assets",
            ZipRoot::Data => "data",
        }
    }
}

/// A verified temp file inside the target directory. Commit it with
/// [`VerifiedDownload::commit`] (or [`commit_verified`]); dropping it
/// without committing removes only the owned file. Unix cleanup uses the retained
/// directory handle, including when the parent has moved.
#[derive(Debug)]
pub struct VerifiedDownload {
    /// The temp file. Lives in the target dir, so the commit is atomic.
    pub temp_path: PathBuf,
    /// SHA-256 of the downloaded bytes, for the provenance record.
    pub sha256: String,
    /// Bytes written.
    pub bytes: u64,
    committed: bool,
    identity: DownloadFileIdentity,
}

impl Drop for VerifiedDownload {
    fn drop(&mut self) {
        if !self.committed {
            self.identity.remove(&self.temp_path);
        }
    }
}

pub(crate) fn directory_handle(path: &Path) -> std::io::Result<same_file::Handle> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)?;
        same_file::Handle::from_file(file)
    }
    #[cfg(not(unix))]
    {
        same_file::Handle::from_path(path)
    }
}

#[derive(Debug)]
pub(crate) struct DownloadFileIdentity {
    parent: same_file::Handle,
    file: same_file::Handle,
}

impl DownloadFileIdentity {
    fn matches(&self, path: &Path) -> bool {
        let Some(dir) = path.parent() else {
            return false;
        };
        std::fs::symlink_metadata(dir).is_ok_and(|entry| entry.is_dir())
            && directory_handle(dir).is_ok_and(|current| current == self.parent)
            && std::fs::symlink_metadata(path).is_ok_and(|entry| entry.is_file())
            && crate::read_capped::open_regular(path)
                .and_then(same_file::Handle::from_file)
                .is_ok_and(|current| current == self.file)
    }

    pub(crate) fn remove(&self, path: &Path) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;

            let Some(name) = path.file_name() else { return };
            let Ok(current) =
                open_regular_at(&self.parent, name).and_then(same_file::Handle::from_file)
            else {
                return;
            };
            if current != self.file {
                return;
            }
            let Ok(name) = download_entry_name(name) else {
                return;
            };
            // SAFETY: parent retains a valid directory fd; name is one NUL-terminated component.
            unsafe { libc::unlinkat(self.parent.as_file().as_raw_fd(), name.as_ptr(), 0) };
        }
        #[cfg(not(unix))]
        if self.matches(path) {
            let _ = std::fs::remove_file(path);
        }
    }

    fn try_clone(&self) -> std::io::Result<Self> {
        Ok(Self {
            parent: same_file::Handle::from_file(self.parent.as_file().try_clone()?)?,
            file: same_file::Handle::from_file(self.file.as_file().try_clone()?)?,
        })
    }
}

#[cfg(unix)]
fn download_entry_name(name: &std::ffi::OsStr) -> std::io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;

    std::ffi::CString::new(name.as_bytes()).map_err(std::io::Error::other)
}

#[cfg(unix)]
fn open_regular_at(
    parent: &same_file::Handle,
    name: &std::ffi::OsStr,
) -> std::io::Result<std::fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};

    let name = download_entry_name(name)?;
    // SAFETY: parent retains a valid directory fd; name is one NUL-terminated component.
    let fd = unsafe {
        libc::openat(
            parent.as_file().as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: openat returned a new fd that this File now owns.
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "download temp is not a regular file",
        ));
    }
    Ok(file)
}

impl VerifiedDownload {
    /// Atomically move the verified temp to `target_dir/filename` without
    /// replacing an existing file. Marks the download committed so `Drop`
    /// no longer deletes it.
    pub fn commit(self, target_dir: &Path, filename: &str) -> Result<PathBuf, DownloadError> {
        self.commit_with_identity(target_dir, filename)
            .map(|(dest, _)| dest)
    }

    pub(crate) fn commit_with_identity(
        mut self,
        target_dir: &Path,
        filename: &str,
    ) -> Result<(PathBuf, DownloadFileIdentity), DownloadError> {
        if !self.identity.matches(&self.temp_path)
            || !directory_handle(target_dir).is_ok_and(|current| current == self.identity.parent)
        {
            return Err(DownloadError::Io {
                reason: "download file or target directory changed since preparation".to_string(),
            });
        }
        let identity = self.identity.try_clone().map_err(|err| DownloadError::Io {
            reason: format!("cannot retain download identity: {err}"),
        })?;
        let dest = commit_path(&self.temp_path, target_dir, filename)?;
        self.committed = true;
        Ok((dest, identity))
    }
}

/// The file name must be a plain name: no separators, no escapes.
pub fn validate_filename(name: &str) -> Result<(), DownloadError> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        return Err(DownloadError::BadFilename {
            name: name.to_string(),
        });
    }
    Ok(())
}

fn temp_path_for(target_dir: &Path, filename: &str) -> PathBuf {
    let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    target_dir.join(format!(
        ".{filename}.mine-utils-download-{pid}-{stamp}-{id}.tmp",
        pid = std::process::id()
    ))
}

#[cfg(unix)]
fn sweep_download_temps(target_dir: &Path) {
    let Ok(parent) = directory_handle(target_dir) else {
        return;
    };
    if !is_local_download_filesystem(&parent) {
        return;
    }
    let Ok(entries) = std::fs::read_dir(target_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some((prefix, suffix)) = name.rsplit_once(".mine-utils-download-") else {
            continue;
        };
        if !prefix.starts_with('.') || prefix.len() < 2 {
            continue;
        }
        let Some(suffix) = suffix.strip_suffix(".tmp") else {
            continue;
        };
        let mut parts = suffix.split('-');
        let (Some(pid), Some(stamp), Some(counter), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if ![pid, stamp, counter]
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        {
            continue;
        }
        let Ok(pid) = pid.parse::<u32>() else {
            continue;
        };
        if pid == std::process::id() {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if !kind.is_file() {
            continue;
        }
        if let Err(err) = sweep_download_temp(&parent, &entry.file_name()) {
            log::debug!("keeping download temp {}: {err}", entry.path().display());
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn is_local_download_filesystem(parent: &same_file::Handle) -> bool {
    use std::os::fd::AsRawFd;
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: parent retains a valid fd and stat points to writable storage.
    if unsafe { libc::fstatfs(parent.as_file().as_raw_fd(), stat.as_mut_ptr()) } != 0 {
        return false;
    }
    // SAFETY: a successful fstatfs initialized stat.
    let stat = unsafe { stat.assume_init() };
    #[cfg(target_os = "linux")]
    return is_local_linux_filesystem(stat.f_type as u32);
    #[cfg(target_os = "macos")]
    return stat.f_flags & libc::MNT_LOCAL as u32 != 0;
}

#[cfg(target_os = "linux")]
fn is_local_linux_filesystem(fs_type: u32) -> bool {
    const RAMFS_MAGIC: u32 = 0x858458f6;

    // Locks on network and FUSE mounts may not exclude owners on another mount.
    // Unknown filesystems stay excluded until their locking semantics are known.
    [
        libc::EXT4_SUPER_MAGIC as u32,
        libc::BTRFS_SUPER_MAGIC as u32,
        libc::XFS_SUPER_MAGIC as u32,
        libc::F2FS_SUPER_MAGIC as u32,
        libc::TMPFS_MAGIC as u32,
        RAMFS_MAGIC,
    ]
    .contains(&fs_type)
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn is_local_download_filesystem(_parent: &same_file::Handle) -> bool {
    false
}

#[cfg(unix)]
fn sweep_download_temp(parent: &same_file::Handle, name: &std::ffi::OsStr) -> std::io::Result<()> {
    use std::time::Duration;

    let file = open_regular_at(parent, name)?;
    let metadata = file.metadata()?;
    lock_download_temp(&file, libc::LOCK_EX | libc::LOCK_NB)?;
    let old = metadata
        .modified()?
        .elapsed()
        .is_ok_and(|age| age > Duration::from_secs(3600));
    if !old {
        return Ok(());
    }
    let identity = DownloadFileIdentity {
        parent: same_file::Handle::from_file(parent.as_file().try_clone()?)?,
        file: same_file::Handle::from_file(file)?,
    };
    identity.remove(Path::new(name));
    Ok(())
}

#[cfg(unix)]
fn lock_download_temp(file: &std::fs::File, operation: libc::c_int) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;

    // SAFETY: file keeps the descriptor valid throughout flock.
    if unsafe { libc::flock(file.as_raw_fd(), operation) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(unix))]
fn sweep_download_temps(_target_dir: &Path) {}

/// Stream a CDN file into a temp file in `target_dir`, checking the 500 MB
/// cap and the SHA-512 while streaming, then validating the zip root.
/// The URL must be `https://cdn.modrinth.com/...` and every redirect hop
/// must stay on the Modrinth hosts. The temp is deleted on every error.
pub fn download_verified<T: Transport + 'static>(
    client: &ModrinthClient<T>,
    url: &str,
    target_dir: &Path,
    filename: &str,
    expected_sha512: &str,
    root: ZipRoot,
) -> Result<VerifiedDownload, DownloadError> {
    download_verified_capped(
        client,
        url,
        target_dir,
        filename,
        expected_sha512,
        root,
        MAX_DOWNLOAD_BYTES,
    )
}

/// [`download_verified`] with an explicit byte cap. Tests use a small cap to
/// prove the limit without staging 500 MB; production passes
/// [`MAX_DOWNLOAD_BYTES`].
pub fn download_verified_capped<T: Transport + 'static>(
    client: &ModrinthClient<T>,
    url: &str,
    target_dir: &Path,
    filename: &str,
    expected_sha512: &str,
    root: ZipRoot,
    max_bytes: u64,
) -> Result<VerifiedDownload, DownloadError> {
    download_verified_controlled(
        client,
        url,
        target_dir,
        filename,
        expected_sha512,
        root,
        max_bytes,
        &InstallControl::default(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn download_verified_controlled<T: Transport + 'static>(
    client: &ModrinthClient<T>,
    url: &str,
    target_dir: &Path,
    filename: &str,
    expected_sha512: &str,
    root: ZipRoot,
    max_bytes: u64,
    control: &InstallControl,
) -> Result<VerifiedDownload, DownloadError> {
    control.check()?;
    validate_filename(filename)?;
    std::fs::create_dir_all(target_dir).map_err(|err| DownloadError::Io {
        reason: format!("cannot use {}: {err}", target_dir.display()),
    })?;
    sweep_download_temps(target_dir);
    let parent = directory_handle(target_dir).map_err(|err| DownloadError::Io {
        reason: format!("cannot open {}: {err}", target_dir.display()),
    })?;
    let temp = temp_path_for(target_dir, filename);
    let temp_file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|err| DownloadError::Io {
            reason: format!("cannot create {}: {err}", temp.display()),
        })?;
    let mut verified = VerifiedDownload {
        temp_path: temp,
        sha256: String::new(),
        bytes: 0,
        committed: false,
        identity: DownloadFileIdentity {
            file: same_file::Handle::from_file(temp_file).map_err(|err| DownloadError::Io {
                reason: format!("cannot identify download temp: {err}"),
            })?,
            parent,
        },
    };
    #[cfg(unix)]
    if let Err(err) = lock_download_temp(verified.identity.file.as_file(), libc::LOCK_EX) {
        if err.raw_os_error().is_some_and(|code| {
            [
                libc::EOPNOTSUPP,
                libc::ENOTSUP,
                libc::ENOLCK,
                libc::ENOSYS,
                libc::EINVAL,
            ]
            .contains(&code)
        }) {
            log::debug!(
                "download temp lock unavailable for {}: {err}",
                verified.temp_path.display()
            );
        } else {
            return Err(DownloadError::Io {
                reason: format!("cannot lock {}: {err}", verified.temp_path.display()),
            });
        }
    }
    let (sha256, bytes) = download_into(
        client,
        url,
        &verified.temp_path,
        verified.identity.file.as_file_mut(),
        expected_sha512,
        root,
        max_bytes,
        control,
    )?;
    verified.sha256 = sha256;
    verified.bytes = bytes;
    Ok(verified)
}

#[allow(clippy::too_many_arguments)]
fn download_into<T: Transport + 'static>(
    client: &ModrinthClient<T>,
    url: &str,
    temp: &Path,
    out: &mut std::fs::File,
    expected_sha512: &str,
    root: ZipRoot,
    max_bytes: u64,
    control: &InstallControl,
) -> Result<(String, u64), DownloadError> {
    let permit = ReaderPermit::acquire(control)?;
    let (tx, rx) = mpsc::sync_channel(1);
    let client = client.clone();
    let url = url.to_string();
    let stream_url = url.clone();
    let worker_control = control.clone();
    // The worker owns only network bytes. The caller alone owns the temp,
    // so cancellation can delete it even while a socket read is blocked.
    std::thread::Builder::new()
        .name("modrinth-reader".into())
        .spawn(move || {
            let _permit = permit;
            let stream = (|| {
                worker_control.check()?;
                Ok(client.file_stream(&stream_url)?.body.take(max_bytes + 1))
            })();
            let mut reader = match stream {
                Ok(reader) => reader,
                Err(err) => {
                    let _ = tx.send(Err(err));
                    return;
                }
            };
            loop {
                if worker_control.check().is_err() {
                    return;
                }
                let mut chunk = vec![0u8; 64 * 1024];
                let result = reader
                    .read(&mut chunk)
                    .map(|n| {
                        chunk.truncate(n);
                        chunk
                    })
                    .map_err(|err| DownloadError::Io {
                        reason: format!("download interrupted: {err}"),
                    });
                let done = result.as_ref().map_or(true, |bytes| bytes.is_empty());
                if tx.send(result).is_err() || done {
                    return;
                }
            }
        })
        .map_err(|err| DownloadError::Io {
            reason: format!("cannot start download worker: {err}"),
        })?;
    let mut sha512 = Sha512::new();
    let mut sha256 = Sha256::new();
    let mut bytes: u64 = 0;
    loop {
        control.check()?;
        let wait = control
            .deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(25));
        let chunk = match rx.recv_timeout(wait) {
            Ok(result) => {
                control.check()?;
                result?
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                control.check()?;
                return Err(DownloadError::Io {
                    reason: "download worker stopped".to_string(),
                });
            }
        };
        let n = chunk.len();
        if n == 0 {
            break;
        }
        bytes += n as u64;
        if bytes > max_bytes {
            return Err(DownloadError::TooLarge { url: url.clone() });
        }
        sha512.update(&chunk[..n]);
        sha256.update(&chunk[..n]);
        out.write_all(&chunk[..n])
            .map_err(|err| DownloadError::Io {
                reason: format!("cannot write {}: {err}", temp.display()),
            })?;
    }
    out.flush().map_err(|err| DownloadError::Io {
        reason: format!("cannot write {}: {err}", temp.display()),
    })?;
    // Fsync the temp before the atomic rename so a crash cannot leave a
    // zero/partial destination that no later check re-hashes. The parent-dir
    // sync in `commit_path` then makes the rename itself durable.
    out.sync_all().map_err(|err| DownloadError::Io {
        reason: format!("cannot write {}: {err}", temp.display()),
    })?;
    let actual_sha512 = hex::encode(sha512.finalize());
    if !constant_time_eq_hex(&actual_sha512, expected_sha512) {
        return Err(DownloadError::HashMismatch {
            expected: expected_sha512.to_ascii_lowercase(),
            actual: actual_sha512,
        });
    }
    validate_zip_root(temp, root)?;
    control.check()?;
    Ok((hex::encode(sha256.finalize()), bytes))
}

fn constant_time_eq_hex(actual: &str, expected: &str) -> bool {
    if actual.len() != expected.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in actual.bytes().zip(expected.to_ascii_lowercase().bytes()) {
        diff |= a ^ b;
    }
    diff == 0 && !actual.is_empty()
}

/// The temp must be a zip containing the expected root. Not retried. Only
/// a bounded number of entry names are kept for the error summary, so a
/// hostile archive with millions of entries cannot blow up the message.
fn validate_zip_root(temp: &Path, root: ZipRoot) -> Result<(), DownloadError> {
    const MAX_LISTED_ENTRIES: usize = 64;
    let file = std::fs::File::open(temp).map_err(|err| DownloadError::Io {
        reason: err.to_string(),
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(|err| DownloadError::NotZip {
        reason: err.to_string(),
    })?;
    if archive.is_empty() {
        return Err(DownloadError::WrongRoot {
            expected: root.dir().to_string(),
            found: Vec::new(),
        });
    }
    let want = format!("{}/", root.dir());
    let mut found = Vec::new();
    let mut truncated = false;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|err| DownloadError::NotZip {
                reason: err.to_string(),
            })?;
        let name = entry.name().to_string();
        if found.len() < MAX_LISTED_ENTRIES {
            found.push(name.clone());
        } else {
            truncated = true;
        }
        if name == root.dir() || name.starts_with(&want) {
            return Ok(());
        }
    }
    if truncated {
        found.push("…".to_string());
    }
    Err(DownloadError::WrongRoot {
        expected: root.dir().to_string(),
        found,
    })
}

fn commit_path(temp: &Path, target_dir: &Path, filename: &str) -> Result<PathBuf, DownloadError> {
    validate_filename(filename)?;
    let dest = target_dir.join(filename);
    atomic_no_replace(temp, &dest).map_err(|err| {
        if err.kind() == std::io::ErrorKind::AlreadyExists {
            DownloadError::Exists {
                name: filename.to_string(),
            }
        } else {
            DownloadError::Io {
                reason: format!("cannot move download into place: {err}"),
            }
        }
    })?;
    sync_parent(&dest);
    Ok(dest)
}

fn sync_parent(path: &Path) {
    if let Some(parent) = path.parent() {
        if let Ok(handle) = std::fs::File::open(parent) {
            let _ = handle.sync_all();
        }
    }
}

/// Atomically rename a verified temp into `target_dir/filename`. Refuses to
/// overwrite: an existing name routes through the duplicate prompt instead.
pub fn commit_verified(
    temp: &Path,
    target_dir: &Path,
    filename: &str,
) -> Result<PathBuf, DownloadError> {
    commit_path(temp, target_dir, filename)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_filenames() {
        for bad in ["", ".", "..", "a/b.zip", "a\\b.zip", "a\0b.zip"] {
            assert!(validate_filename(bad).is_err(), "{bad:?} must be refused");
        }
        assert!(validate_filename("Fresh Animations v1.9.zip").is_ok());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn only_known_local_filesystems_allow_sweeping() {
        for local in [
            0xef53, 0x9123683e, 0x58465342, 0xf2f52010, 0x01021994, 0x858458f6, 0x2fc12fc1,
        ] {
            assert!(is_local_linux_filesystem(local), "{local:#x}");
        }
        for (name, fs_type) in [
            ("NFS", 0x6969),
            ("SMB", 0x517b),
            ("CIFS", 0xff534d42),
            ("SMB2", 0xfe534d42),
            ("FUSE including SSHFS", 0x65735546),
            ("9P", 0x01021997),
            ("AFS", 0x5346414f),
            ("kAFS", 0x6b414653),
            ("Ceph", 0x00c36400),
            ("Lustre", 0x0bd00bd0),
            ("GFS2", 0x01161970),
            ("OCFS2", 0x7461636f),
            ("overlay with unknown backing filesystem", 0x794c7630),
            ("unknown", 0x12345678),
            ("zero", 0),
            ("all bits set", u32::MAX),
        ] {
            assert!(!is_local_linux_filesystem(fs_type), "{name}: {fs_type:#x}");
        }
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    #[test]
    fn rollback_removes_only_the_committed_download() {
        for swap in ["none", "file", "parent"] {
            let root = tempfile::tempdir().unwrap();
            let target = root.path().join("resourcepacks");
            std::fs::create_dir(&target).unwrap();
            let temp_path = target.join(".pack.tmp");
            let file = std::fs::File::create(&temp_path).unwrap();
            (&file).write_all(b"downloaded bytes").unwrap();
            let verified = VerifiedDownload {
                temp_path,
                sha256: String::new(),
                bytes: 16,
                committed: false,
                identity: DownloadFileIdentity {
                    parent: directory_handle(&target).unwrap(),
                    file: same_file::Handle::from_file(file).unwrap(),
                },
            };
            let (dest, identity) = verified.commit_with_identity(&target, "pack.zip").unwrap();
            let moved = root.path().join("moved");
            let original = match swap {
                "file" => {
                    std::fs::rename(&dest, &moved).unwrap();
                    std::fs::write(&dest, b"unrelated sentinel").unwrap();
                    moved
                }
                "parent" => {
                    std::fs::rename(&target, &moved).unwrap();
                    std::fs::create_dir(&target).unwrap();
                    std::fs::write(&dest, b"unrelated sentinel").unwrap();
                    moved.join("pack.zip")
                }
                _ => dest.clone(),
            };
            identity.remove(&dest);
            if swap == "none" {
                assert!(!dest.exists());
            } else {
                assert_eq!(
                    std::fs::read(dest).unwrap(),
                    b"unrelated sentinel",
                    "{swap}"
                );
                if cfg!(unix) && swap == "parent" {
                    assert!(!original.exists());
                } else {
                    assert_eq!(
                        std::fs::read(original).unwrap(),
                        b"downloaded bytes",
                        "{swap}"
                    );
                }
            }
        }
    }
}
